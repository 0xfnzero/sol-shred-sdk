//! Cryptographic transaction-signature and wire/Entry ingestion regressions.
//! Synthetic instructions test extraction; only captured mainnet cases establish
//! execution. Raw shreds do not provide CPI execution metadata or prove success.
#[path = "../examples/common/mod.rs"]
mod common;
#[path = "../examples/common/pumpswap_verify.rs"]
mod pumpswap_verify;
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::{
    hash::Hash,
    instr::{program_ids::PUMPSWAP_PROGRAM_ID, pump_amm::discriminators},
    message::VersionedMessage,
    shred::{entries_to_transactions, entries_to_tx_batch},
    transaction::VersionedTransaction,
    DexEvent, EventType, EventTypeFilter, Pubkey, RawShredConfig, RawShredDecoder,
};
use solana_entry::entry::Entry;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{Keypair, Signer};
use solana_ledger::shred::{ProcessShredsStats, ReedSolomonCache, Shredder};
use solana_message::{v0, AddressLookupTableAccount};
use std::{fs, path::Path, time::Instant};

fn verified(tx: &VersionedTransaction) -> bool {
    // Check cardinality explicitly: zip/all alone would accept missing signatures.
    let required = tx.message.header().num_required_signatures as usize;
    required != 0
        && tx.signatures.len() == required
        && tx.message.static_account_keys().len() >= required
        && tx
            .signatures
            .iter()
            .zip(tx.message.static_account_keys())
            .all(|(sig, key)| sig.verify(key.as_ref(), &tx.message.serialize()))
}

fn compact(
    user: &Keypair,
    tag: [u8; 8],
    seed: u8,
    first: u64,
    second: u64,
) -> (Instruction, Vec<Pubkey>) {
    let mut keys: Vec<_> = (0..17)
        .map(|i| Pubkey::new_from_array([seed + i; 32]))
        .collect();
    keys[1] = user.pubkey();
    keys[9] = sol_shred_sdk::accounts::program_ids::SPL_TOKEN_PROGRAM_ID;
    keys[10] = sol_shred_sdk::accounts::program_ids::SPL_TOKEN_PROGRAM_ID;
    keys[11] = Pubkey::default();
    keys[16] = PUMPSWAP_PROGRAM_ID;
    let mut data = tag.to_vec();
    data.extend_from_slice(&first.to_le_bytes());
    data.extend_from_slice(&second.to_le_bytes());
    let ix = Instruction {
        program_id: PUMPSWAP_PROGRAM_ID,
        accounts: keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                if i == 1 {
                    AccountMeta::new(*k, true)
                } else {
                    AccountMeta::new_readonly(*k, false)
                }
            })
            .collect(),
        data,
    };
    (ix, keys)
}

fn signed(
    payer: &Keypair,
    user: &Keypair,
    ix: &[Instruction],
    alt: &[AddressLookupTableAccount],
    legacy: bool,
) -> VersionedTransaction {
    let blockhash = Hash::new_from_array([77; 32]);
    let message = if legacy {
        VersionedMessage::Legacy(solana_message::Message::new_with_blockhash(
            ix,
            Some(&payer.pubkey()),
            &blockhash,
        ))
    } else {
        VersionedMessage::V0(v0::Message::try_compile(&payer.pubkey(), ix, alt, blockhash).unwrap())
    };
    let tx = VersionedTransaction::try_new(message, &[payer, user]).unwrap();
    assert!(verified(&tx));
    tx
}

fn resolve(
    tx: &VersionedTransaction,
    tables: &[AddressLookupTableAccount],
) -> (Vec<Pubkey>, Vec<Pubkey>) {
    let mut writable = vec![];
    let mut readonly = vec![];
    for lookup in tx.message.address_table_lookups().unwrap_or_default() {
        let table = tables.iter().find(|t| t.key == lookup.account_key).unwrap();
        writable.extend(
            lookup
                .writable_indexes
                .iter()
                .map(|i| table.addresses[*i as usize]),
        );
        readonly.extend(
            lookup
                .readonly_indexes
                .iter()
                .map(|i| table.addresses[*i as usize]),
        );
    }
    (writable, readonly)
}

fn parse(
    tx: &VersionedTransaction,
    loaded: &(Vec<Pubkey>, Vec<Pubkey>),
    filter: Option<&EventTypeFilter>,
) -> Vec<DexEvent> {
    let mut events = vec![];
    sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
        tx,
        &loaded.0,
        &loaded.1,
        tx.signatures[0],
        42,
        3,
        999,
        filter,
        &mut events,
    )
    .unwrap();
    for event in &events {
        let m = event.metadata();
        assert_eq!(m.signature, tx.signatures[0]);
        assert_eq!(m.slot, 42);
        assert_eq!(m.tx_index, 3);
        assert_eq!(m.grpc_recv_us, 999);
        assert_eq!(
            m.recent_blockhash,
            Some(tx.message.recent_blockhash().to_string())
        );
    }
    events
}

#[test]
fn real_ed25519_multisigner_wire_compact_and_alt_preserve_each_pool() {
    // Public deterministic test keys; these never represent funded wallets.
    let payer = Keypair::new_from_array([9; 32]);
    let user = Keypair::new_from_array([10; 32]);
    let (buy, a) = compact(&user, discriminators::BUY_V2, 20, u64::MAX, 17);
    let (exact, b) = compact(&user, discriminators::BUY_EXACT_QUOTE_IN_V2, 50, 101, 23);
    let (sell, c) = compact(&user, discriminators::SELL_V2, 80, 31, 29);
    let instructions = [buy, exact, sell];
    let table = AddressLookupTableAccount {
        key: Pubkey::new_from_array([111; 32]),
        addresses: a.iter().chain(&b).chain(&c).copied().collect(),
    };
    for (legacy, use_alt) in [(true, false), (false, false), (false, true)] {
        let tables = if use_alt {
            std::slice::from_ref(&table)
        } else {
            &[]
        };
        let tx = signed(&payer, &user, &instructions, tables, legacy);
        assert_eq!(tx.signatures.len(), 2);
        let wire = wincode::serialize(&tx).unwrap();
        let decoded: VersionedTransaction = wincode::deserialize_exact(&wire).unwrap();
        assert_eq!(decoded, tx);
        assert!(verified(&decoded));
        assert!(wincode::deserialize_exact::<VersionedTransaction>(
            &[wire.as_slice(), &[0]].concat()
        )
        .is_err());
        for length in 0..wire.len() {
            assert!(wincode::deserialize_exact::<VersionedTransaction>(&wire[..length]).is_err());
        }
        let events = parse(&decoded, &resolve(&decoded, tables), None);
        assert_eq!(events.len(), 3);
        let DexEvent::PumpSwapBuy(first) = &events[0] else {
            panic!("buy")
        };
        let DexEvent::PumpSwapBuy(second) = &events[1] else {
            panic!("exact quote")
        };
        let DexEvent::PumpSwapSell(third) = &events[2] else {
            panic!("sell")
        };
        assert_eq!(
            (
                first.pool,
                first.user,
                first.base_amount_out,
                first.max_quote_amount_in
            ),
            (a[0], user.pubkey(), u64::MAX, 17)
        );
        assert_eq!(
            (
                second.pool,
                second.base_amount_out,
                second.max_quote_amount_in,
                second.min_base_amount_out
            ),
            (b[0], 23, 101, 23)
        );
        assert_eq!(
            (
                third.pool,
                third.user,
                third.base_amount_in,
                third.min_quote_amount_out
            ),
            (c[0], user.pubkey(), 31, 29)
        );
        for (event, keys) in events.iter().zip([&a, &b, &c]) {
            match event {
                DexEvent::PumpSwapBuy(e) => assert_eq!(
                    (
                        e.base_mint,
                        e.quote_mint,
                        e.user_base_token_account,
                        e.user_quote_token_account
                    ),
                    (keys[3], keys[4], keys[5], keys[6])
                ),
                DexEvent::PumpSwapSell(e) => assert_eq!(
                    (
                        e.base_mint,
                        e.quote_mint,
                        e.user_base_token_account,
                        e.user_quote_token_account
                    ),
                    (keys[3], keys[4], keys[5], keys[6])
                ),
                _ => unreachable!(),
            }
        }
        let filter = EventTypeFilter::include_only(vec![EventType::PumpSwapSell]);
        assert_eq!(
            parse(&decoded, &resolve(&decoded, tables), Some(&filter)).len(),
            1
        );
        if use_alt {
            let mut unresolved = vec![];
            sol_shred_sdk::parse_transaction_dex_events(
                &decoded,
                decoded.signatures[0],
                42,
                3,
                999,
                &mut unresolved,
            );
            assert!(unresolved.is_empty());
            // Bad resolver inputs fail atomically, including when a caller reuses its buffer.
            let mut existing = events.clone();
            let before = serde_json::to_string(&existing).unwrap();
            assert!(
                sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
                    &decoded,
                    &[],
                    &[],
                    decoded.signatures[0],
                    42,
                    3,
                    999,
                    None,
                    &mut existing
                )
                .is_err()
            );
            assert_eq!(serde_json::to_string(&existing).unwrap(), before);
        }
        // A correctly signed malformed middle instruction must fail the resolved
        // entry atomically, even with valid instructions on either side.
        for invalid_program in [false, true] {
            let mut message = decoded.message.clone();
            let instructions = match &mut message {
                VersionedMessage::Legacy(m) => &mut m.instructions,
                VersionedMessage::V0(m) => &mut m.instructions,
                _ => unreachable!(),
            };
            if invalid_program {
                instructions[1].program_id_index = 255;
            } else {
                instructions[1].accounts[0] = 255;
            }
            let malformed = VersionedTransaction::try_new(message, &[&payer, &user]).unwrap();
            assert!(verified(&malformed));
            let loaded = resolve(&decoded, tables);
            let mut existing = events.clone();
            let before = serde_json::to_string(&existing).unwrap();
            assert!(
                sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
                    &malformed,
                    &loaded.0,
                    &loaded.1,
                    malformed.signatures[0],
                    42,
                    3,
                    999,
                    None,
                    &mut existing,
                )
                .is_err()
            );
            assert_eq!(serde_json::to_string(&existing).unwrap(), before);
            if !use_alt {
                let mut provisional = vec![];
                sol_shred_sdk::parse_transaction_dex_events(
                    &malformed,
                    malformed.signatures[0],
                    42,
                    3,
                    999,
                    &mut provisional,
                );
                assert_eq!(provisional.len(), 2);
                assert!(matches!(&provisional[0], DexEvent::PumpSwapBuy(e) if e.pool == a[0]));
                assert!(matches!(&provisional[1], DexEvent::PumpSwapSell(e) if e.pool == c[0]));
            }
        }
        let mut bad_signature = decoded.clone();
        bad_signature.signatures[0] = Default::default();
        assert!(!verified(&bad_signature));
        let mut missing = decoded.clone();
        missing.signatures.pop();
        assert!(!verified(&missing));
        let mut changed = decoded.clone();
        match &mut changed.message {
            VersionedMessage::Legacy(m) => m.instructions[0].data[8] ^= 1,
            VersionedMessage::V0(m) => m.instructions[0].data[8] ^= 1,
            _ => unreachable!(),
        };
        assert!(!verified(&changed));
    }
}

#[test]
fn signed_two_tables_group_writable_before_readonly_and_reject_counts_atomically() {
    let payer = Keypair::new_from_array([9; 32]);
    let user = Keypair::new_from_array([10; 32]);
    let (mut buy, a) = compact(&user, discriminators::BUY_V2, 20, 17, 19);
    let (mut sell, b) = compact(&user, discriminators::SELL_V2, 50, 23, 29);
    // Each table has both access classes. Interleaving per-table keys would
    // incorrectly attribute the second pool to a readonly key from table one.
    for ix in [&mut buy, &mut sell] {
        for index in [0, 5, 6] {
            ix.accounts[index].is_writable = true;
        }
    }
    let tables = [
        AddressLookupTableAccount {
            key: Pubkey::new_from_array([111; 32]),
            addresses: a.clone(),
        },
        AddressLookupTableAccount {
            key: Pubkey::new_from_array([112; 32]),
            addresses: b.clone(),
        },
    ];
    let tx = signed(&payer, &user, &[buy, sell], &tables, false);
    let wire = wincode::serialize(&tx).unwrap();
    let decoded: VersionedTransaction = wincode::deserialize_exact(&wire).unwrap();
    decoded.sanitize().unwrap();
    decoded.verify_and_hash_message().unwrap();
    let lookups = decoded.message.address_table_lookups().unwrap();
    assert_eq!(lookups.len(), 2);
    assert!(lookups
        .iter()
        .all(|l| !l.writable_indexes.is_empty() && !l.readonly_indexes.is_empty()));
    let loaded = resolve(&decoded, &tables);
    let events = parse(&decoded, &loaded, None);
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], DexEvent::PumpSwapBuy(e)
        if e.pool == a[0] && e.user_base_token_account == a[5]
        && e.base_mint == a[3] && e.max_quote_amount_in == 19));
    assert!(matches!(&events[1], DexEvent::PumpSwapSell(e)
        if e.pool == b[0] && e.user_base_token_account == b[5]
        && e.base_mint == b[3] && e.min_quote_amount_out == 29));
    let filter = EventTypeFilter::include_only(vec![EventType::PumpSwapSell]);
    for access_class in 0..2 {
        for extra in [false, true] {
            let mut invalid = loaded.clone();
            let addresses = if access_class == 0 {
                &mut invalid.0
            } else {
                &mut invalid.1
            };
            if extra {
                addresses.push(Pubkey::new_from_array([113; 32]));
            } else {
                addresses.pop();
            }
            let mut reused = events.clone();
            let before = serde_json::to_string(&reused).unwrap();
            assert!(
                sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
                    &decoded,
                    &invalid.0,
                    &invalid.1,
                    decoded.signatures[0],
                    42,
                    3,
                    999,
                    Some(&filter),
                    &mut reused,
                )
                .is_err()
            );
            assert_eq!(serde_json::to_string(&reused).unwrap(), before);
        }
    }
    // Successful append leaves old event metadata untouched even with a filter.
    let mut reused = events.clone();
    sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
        &decoded,
        &loaded.0,
        &loaded.1,
        decoded.signatures[0],
        43,
        4,
        1000,
        Some(&filter),
        &mut reused,
    )
    .unwrap();
    assert_eq!(reused.len(), 3);
    assert_eq!(reused[0].metadata().slot, 42);
    assert_eq!(reused[1].metadata().grpc_recv_us, 999);
    assert_eq!(reused[2].metadata().slot, 43);
    assert_eq!(reused[2].metadata().grpc_recv_us, 1000);
}

#[test]
fn signed_compact_malformed_lengths_and_account_tails_keep_valid_neighbor() {
    let payer = Keypair::new_from_array([9; 32]);
    let user = Keypair::new_from_array([10; 32]);
    for tag in [
        discriminators::BUY_V2,
        discriminators::BUY_EXACT_QUOTE_IN_V2,
        discriminators::SELL_V2,
    ] {
        let (valid, keys) = compact(&user, tag, 20, 17, 19);
        for legacy in [true, false] {
            for boundary in 0..4 {
                let mut malformed = valid.clone();
                match boundary {
                    0 => {
                        malformed.data.pop();
                    }
                    1 => malformed.data.push(0),
                    2 => {
                        malformed.accounts.pop();
                    }
                    3 => malformed.accounts.push(malformed.accounts[0].clone()),
                    _ => unreachable!(),
                }
                let tx = signed(
                    &payer,
                    &user,
                    &[valid.clone(), malformed, valid.clone()],
                    &[],
                    legacy,
                );
                let events = parse(&tx, &(vec![], vec![]), None);
                assert_eq!(events.len(), 2, "compact ABI boundary={boundary}");
                for e in events {
                    match e {
                        DexEvent::PumpSwapBuy(e) => assert_eq!(e.pool, keys[0]),
                        DexEvent::PumpSwapSell(e) => assert_eq!(e.pool, keys[0]),
                        _ => panic!("unexpected compact variant"),
                    }
                }
            }
        }
    }
}

#[test]
fn signed_customizable_clmm_rejects_missing_fields_invalid_flags_and_short_accounts() {
    use solana_instruction::AccountMeta;
    let payer = Keypair::new_from_array([9; 32]);
    let user = Keypair::new_from_array([10; 32]);
    let keys: Vec<_> = (20..33).map(|n| Pubkey::new_from_array([n; 32])).collect();
    let mut data =
        sol_shred_sdk::instr::raydium_clmm::discriminators::CREATE_CUSTOMIZABLE_POOL.to_vec();
    data.extend_from_slice(&u128::MAX.to_le_bytes());
    data.extend_from_slice(&[2, 1]);
    let valid = Instruction {
        program_id: sol_shred_sdk::instr::program_ids::RAYDIUM_CLMM_PROGRAM_ID,
        accounts: keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                if i == 0 {
                    AccountMeta::new(user.pubkey(), true)
                } else {
                    AccountMeta::new_readonly(*k, false)
                }
            })
            .collect(),
        data,
    };
    for boundary in 0..6 {
        let mut bad = valid.clone();
        match boundary {
            0 => bad.data.truncate(24),
            1 => bad.data.truncate(25),
            2 => bad.data[24] = 3,
            3 => bad.data[25] = 2,
            4 => {
                bad.accounts.pop();
            }
            5 => bad.data.push(0),
            _ => unreachable!(),
        }
        let tx = signed(
            &payer,
            &user,
            &[valid.clone(), bad, valid.clone()],
            &[],
            false,
        );
        let events = parse(&tx, &(vec![], vec![]), None);
        assert_eq!(events.len(), 2, "customizable CLMM boundary={boundary}");
        for event in events {
            assert!(matches!(event, DexEvent::RaydiumClmmCreatePool(e)
                if e.pool == keys[2] && e.creator == user.pubkey()
                && e.sqrt_price_x64 == u128::MAX));
        }
    }
}

#[test]
fn signed_compact_transactions_survive_real_merkle_shred_reassembly_and_fec() {
    let payer = Keypair::new_from_array([9; 32]);
    let user = Keypair::new_from_array([10; 32]);
    let (buy, keys) = compact(&user, discriminators::BUY_V2, 20, 17, 19);
    let tx = signed(&payer, &user, &[buy], &[], false);
    let entries = vec![Entry {
        num_hashes: 1,
        hash: solana_entry::entry::next_hash(&Hash::default(), 1, &[tx.clone()]),
        transactions: vec![tx.clone()],
    }];
    assert!(solana_entry::entry::EntryVerificationData::from(&entries[0]).verify(&Hash::default()));
    let leader = Keypair::new_from_array([12; 32]);
    let (data, coding) = Shredder::new(42, 41, 0, 0)
        .unwrap()
        .entries_to_merkle_shreds_for_tests(
            &leader,
            &entries,
            true,
            Hash::default(),
            0,
            0,
            &ReedSolomonCache::default(),
            &mut ProcessShredsStats::default(),
        );
    assert!(!data.is_empty());
    assert!(!coding.is_empty());
    for recover in [false, true] {
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let mut batches = vec![];
        let now = Instant::now();
        assert!(decoder.push_packet(&[0; 7], now).is_empty());
        let packets: Vec<_> = if recover {
            data.iter().skip(1).chain(coding.iter()).collect()
        } else {
            data.iter().rev().collect()
        };
        for packet in packets {
            assert!(packet.verify(&leader.pubkey()));
            batches.extend(decoder.push_packet(packet.payload(), now));
            assert!(decoder.push_packet(packet.payload(), now).is_empty());
        }
        // Replaying all packets after completion cannot emit a second transaction.
        for packet in data.iter().chain(coding.iter()) {
            assert!(decoder.push_packet(packet.payload(), now).is_empty());
        }
        assert!(!batches.is_empty());
        let mut decoded = vec![];
        for batch in batches {
            assert_eq!(batch.slot, 42);
            assert_eq!(entries_to_transactions(&batch.entries).len(), 1);
            decoded.extend(entries_to_tx_batch(batch).transactions);
        }
        assert_eq!(decoded, vec![tx.clone()]);
        assert!(verified(&decoded[0]));
        let e = parse(&decoded[0], &(vec![], vec![]), None);
        assert!(
            matches!(&e[0],DexEvent::PumpSwapBuy(e) if e.pool==keys[0] && e.base_amount_out==17)
        );
        if recover {
            assert!(decoder.stats().fec_recovered_data_shreds > 0);
        }
        assert!(decoder.stats().duplicate_shreds > 0);
    }
}

#[test]
fn captured_mainnet_transaction_signatures_verify_before_public_parser_replay() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    let mut event_count = 0;
    let mut cpi_legs = 0;
    let mut balance_checks = 0;
    let mut log_cpi_pairs = 0;
    let mut protocols = std::collections::HashSet::new();
    let mut failed_status_cases = 0;
    for directory in ["mainnet", "pumpswap_rpc"] {
        for entry in fs::read_dir(root.join(directory)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|s| s != "json") {
                continue;
            }
            let j: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            let Some(encoded) = j["transaction"][0].as_str() else {
                continue;
            };
            let tx: VersionedTransaction =
                wincode::deserialize_exact(&STANDARD.decode(encoded).unwrap()).unwrap();
            assert!(
                verified(&tx),
                "signature verification failed: {}",
                path.display()
            );
            let load = |name| {
                j["meta"]["loadedAddresses"][name]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().parse().unwrap())
                    .collect::<Vec<Pubkey>>()
            };
            event_count += parse(&tx, &(load("writable"), load("readonly")), None).len();
            if directory == "mainnet" {
                let (_, route, balances) = common::validate(&j).unwrap();
                for leg in &route.legs {
                    protocols.insert(format!("{:?}", leg.protocol));
                    assert_ne!(leg.pool, Pubkey::default());
                    assert_ne!(leg.input_account, leg.output_account);
                }
                if !route.legs.is_empty() {
                    // Status metadata is not part of the signed message. A valid
                    // transaction signature must not imply successful execution.
                    let mut failed = j.clone();
                    failed["meta"]["err"] =
                        serde_json::json!({"InstructionError":[0,"InvalidArgument"]});
                    let (_, failed_route, _) = common::validate(&failed).unwrap();
                    assert!(!failed_route.succeeded);
                    assert_eq!(failed_route.legs.len(), route.legs.len());
                    assert!(failed_route
                        .legs
                        .iter()
                        .all(|leg| leg.actual_input_amount.is_none()
                            && leg.actual_output_amount.is_none()));
                    assert!(verified(&tx));
                    failed_status_cases += 1;
                }
                cpi_legs += route
                    .legs
                    .iter()
                    .filter(|l| l.position.inner_index.is_some())
                    .count();
                balance_checks += balances;
            } else {
                log_cpi_pairs += pumpswap_verify::verify(&j).unwrap().checked_log_cpi_pairs;
            }
            checked += 1;
        }
    }
    let upgrade: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_upgrade/mainnet.json")).unwrap();
    for case in upgrade["cases"].as_array().unwrap() {
        let j = &case["encoded"];
        let tx: VersionedTransaction = wincode::deserialize_exact(
            &STANDARD
                .decode(j["transaction"][0].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert!(verified(&tx), "{}", case["signature"]);
        assert_eq!(
            tx.signatures[0].to_string(),
            case["signature"].as_str().unwrap()
        );
        let load = |name| {
            j["meta"]["loadedAddresses"][name]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().parse().unwrap())
                .collect::<Vec<Pubkey>>()
        };
        event_count += parse(&tx, &(load("writable"), load("readonly")), None).len();
        let (_, route, balances) = common::validate(j).unwrap();
        cpi_legs += route
            .legs
            .iter()
            .filter(|l| l.position.inner_index.is_some())
            .count();
        balance_checks += balances;
        checked += 1;
    }
    assert!(checked >= 10);
    assert!(event_count >= 10);
    assert!(cpi_legs > 0);
    assert!(balance_checks > 0);
    assert!(log_cpi_pairs > 0);
    assert!(protocols.len() >= 4);
    assert!(failed_status_cases >= 4);
    println!("verified {checked} captured mainnet Ed25519 transactions; replayed {event_count} public outer events, {cpi_legs} CPI legs, {balance_checks} balance checks, {log_cpi_pairs} log/CPI pairs");
    println!("verified {} historical swap protocols and {failed_status_cases} failure-status boundary cases", protocols.len());
}
