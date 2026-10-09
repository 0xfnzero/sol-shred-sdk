//! Leader-signed shred rejection must preserve exact, transaction-signed neighbors.
//! These are offline extraction fixtures, not proofs of program execution.
use sol_shred_sdk::hash::Hash;
use sol_shred_sdk::{
    instr::{program_ids::PUMPSWAP_PROGRAM_ID, pump_amm::discriminators},
    message::VersionedMessage,
    shred::entries_to_transactions,
    transaction::VersionedTransaction,
    DexEvent, Pubkey, RawShredConfig, RawShredDecoder,
};
use solana_entry::entry::Entry;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{Keypair, Signer};
use solana_ledger::shred::{ProcessShredsStats, ReedSolomonCache, Shred, Shredder};
use solana_message::{v0, AddressLookupTableAccount};
use std::time::Instant;

fn transaction(amount: u64, repeats: usize) -> VersionedTransaction {
    let payer = Keypair::new_from_array([110; 32]);
    let user = Keypair::new_from_array([111; 32]);
    let mut keys: Vec<_> = (20..37)
        .map(|seed| Pubkey::new_from_array([seed; 32]))
        .collect();
    keys[1] = user.pubkey();
    keys[9] = sol_shred_sdk::accounts::program_ids::SPL_TOKEN_PROGRAM_ID;
    keys[10] = keys[9];
    keys[11] = Pubkey::default();
    keys[16] = PUMPSWAP_PROGRAM_ID;
    let mut data = discriminators::BUY_V2.to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&u64::MAX.to_le_bytes());
    let ix = Instruction {
        program_id: PUMPSWAP_PROGRAM_ID,
        accounts: keys
            .iter()
            .enumerate()
            .map(|(i, key)| {
                if i == 1 {
                    AccountMeta::new(*key, true)
                } else {
                    AccountMeta::new_readonly(*key, false)
                }
            })
            .collect(),
        data,
    };
    let table = AddressLookupTableAccount {
        key: Pubkey::new_from_array([90; 32]),
        addresses: keys.clone(),
    };
    let message = v0::Message::try_compile(
        &payer.pubkey(),
        &vec![ix; repeats],
        &[table],
        Hash::new_from_array([77; 32]),
    )
    .unwrap();
    assert!(!message.address_table_lookups.is_empty());
    let tx =
        VersionedTransaction::try_new(VersionedMessage::V0(message), &[&payer, &user]).unwrap();
    assert_signed(&tx);
    tx
}

fn assert_signed(tx: &VersionedTransaction) {
    assert_eq!(tx.signatures.len(), 2);
    assert_eq!(tx.message.header().num_required_signatures, 2);
    for (sig, key) in tx.signatures.iter().zip(tx.message.static_account_keys()) {
        assert!(sig.verify(key.as_ref(), &tx.message.serialize()));
    }
}

fn signed_entry(tx: &VersionedTransaction) -> Entry {
    Entry {
        num_hashes: 1,
        hash: solana_entry::entry::next_hash(&Hash::default(), 1, &[tx.clone()]),
        transactions: vec![tx.clone()],
    }
}

fn shreds(tx: &VersionedTransaction, slot: u64) -> (Vec<Shred>, Vec<Shred>) {
    let entries = vec![signed_entry(tx)];
    assert!(solana_entry::entry::EntryVerificationData::from(&entries[0]).verify(&Hash::default()));
    let leader = Keypair::new_from_array([112; 32]);
    let result = Shredder::new(slot, slot - 1, 0, 0)
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
    for packet in result.0.iter().chain(&result.1) {
        assert!(packet.verify(&leader.pubkey()));
    }
    result
}

fn assert_exact_events(tx: &VersionedTransaction, amount: u64, count: usize) {
    // Resolve the test table independently in canonical lookup order.
    let mut addresses: Vec<_> = (20..37)
        .map(|seed| Pubkey::new_from_array([seed; 32]))
        .collect();
    addresses[1] = Keypair::new_from_array([111; 32]).pubkey();
    addresses[9] = sol_shred_sdk::accounts::program_ids::SPL_TOKEN_PROGRAM_ID;
    addresses[10] = addresses[9];
    addresses[11] = Pubkey::default();
    addresses[16] = PUMPSWAP_PROGRAM_ID;
    let lookup = &tx.message.address_table_lookups().unwrap()[0];
    let writable: Vec<_> = lookup
        .writable_indexes
        .iter()
        .map(|i| addresses[*i as usize])
        .collect();
    let readonly: Vec<_> = lookup
        .readonly_indexes
        .iter()
        .map(|i| addresses[*i as usize])
        .collect();
    let mut events = vec![];
    sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
        tx,
        &writable,
        &readonly,
        tx.signatures[0],
        42,
        0,
        1,
        None,
        &mut events,
    )
    .unwrap();
    assert_eq!(events.len(), count);
    for event in events {
        assert!(matches!(event, DexEvent::PumpSwapBuy(e)
            if e.pool == addresses[0] && e.base_amount_out == amount
            && e.max_quote_amount_in == u64::MAX && e.metadata.signature == tx.signatures[0]));
    }
}

#[test]
fn signed_conflicting_data_and_bad_coding_cannot_replace_valid_transaction() {
    let tx = transaction(17, 1);
    let other = transaction(19, 1);
    let (data, coding) = shreds(&tx, 42);
    let (conflicting, _) = shreds(&other, 42);
    assert!(data.len() > 1);
    let now = Instant::now();
    let mut decoder = RawShredDecoder::new(RawShredConfig::default());
    assert!(decoder.push_packet(data[1].payload(), now).is_empty());
    assert!(decoder.push_packet(data[1].payload(), now).is_empty());
    // Both alternatives have valid leader signatures, but the first accepted
    // same-index payload must win. No mixed transaction is allowed to escape.
    assert_ne!(data[1].payload(), conflicting[1].payload());
    assert!(decoder
        .push_packet(conflicting[1].payload(), now)
        .is_empty());
    assert_eq!(decoder.stats().conflicting_shreds, 1);
    assert_eq!(decoder.stats().duplicate_shreds, 1);
    for packet in data.iter().skip(2) {
        assert!(decoder.push_packet(packet.payload(), now).is_empty());
    }
    let mut bad = coding[0].payload().as_ref().to_vec();
    bad[89] ^= 1; // First erasure payload byte, after the official coding header.
    let corrupted = Shred::new_from_serialized_shred(bad.clone()).unwrap();
    assert!(!corrupted.verify(&Keypair::new_from_array([112; 32]).pubkey()));
    assert!(decoder.push_packet(&bad, now).is_empty());
    assert_eq!(decoder.stats().fec_recover_attempts, 1);
    assert_eq!(decoder.stats().fec_recover_failures, 1);
    assert_eq!(decoder.stats().fec_recovered_data_shreds, 0);
    assert_eq!(decoder.stats().emitted_transactions, 0);
    // A failed FEC set does not spin repeatedly; the complete authentic data
    // path still works and can recover the signed transaction exactly once.
    for packet in coding.iter().skip(1) {
        assert!(decoder.push_packet(packet.payload(), now).is_empty());
    }
    assert_eq!(decoder.stats().fec_recover_attempts, 1);
    let batches = decoder.push_packet(data[0].payload(), now);
    let decoded: Vec<_> = batches
        .iter()
        .flat_map(|b| entries_to_transactions(&b.entries))
        .collect();
    assert_eq!(decoded, vec![&tx]);
    assert_signed(decoded[0]);
    assert_exact_events(decoded[0], 17, 1);
    for packet in data.iter().chain(&coding) {
        assert!(decoder.push_packet(packet.payload(), now).is_empty());
    }
    assert_eq!(decoder.stats().emitted_transactions, 1);
}

#[test]
fn signed_oversized_segment_is_dropped_once_and_next_slot_remains_usable() {
    let small = transaction(23, 1);
    let large = transaction(29, 3);
    let small_entries = vec![signed_entry(&small)];
    let large_entries = vec![signed_entry(&large)];
    let cap = wincode::serialize(&small_entries).unwrap().len();
    assert!(wincode::serialize(&large_entries).unwrap().len() > cap);
    let (large_data, large_code) = shreds(&large, 42);
    let (small_data, _) = shreds(&small, 43);
    let mut decoder = RawShredDecoder::new(RawShredConfig {
        max_deshred_bytes: cap,
        max_tracked_slots: 2,
        ..Default::default()
    });
    let now = Instant::now();
    for packet in &large_data {
        assert!(decoder.push_packet(packet.payload(), now).is_empty());
    }
    assert_eq!(decoder.stats().oversized_payloads, 1);
    assert_eq!(decoder.stats().emitted_transactions, 0);
    for packet in large_data.iter().chain(&large_code) {
        assert!(decoder.push_packet(packet.payload(), now).is_empty());
    }
    assert_eq!(decoder.stats().oversized_payloads, 1);
    let mut batches = vec![];
    for packet in small_data.iter().rev() {
        batches.extend(decoder.push_packet(packet.payload(), now));
    }
    let decoded: Vec<_> = batches
        .iter()
        .flat_map(|b| entries_to_transactions(&b.entries))
        .collect();
    assert_eq!(decoded, vec![&small]);
    assert_signed(decoded[0]);
    assert_exact_events(decoded[0], 23, 1);
    assert_eq!(decoder.stats().emitted_transactions, 1);
}
