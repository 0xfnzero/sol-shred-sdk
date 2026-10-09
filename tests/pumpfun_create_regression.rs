#[path = "support/pumpfun_rpc.rs"]
mod support;
use sol_shred_sdk::{parse_transaction_dex_events_with_loaded_addresses, DexEvent};

use sol_shred_sdk::{pubkey::Pubkey, signature::Signature};
use std::{fs, path::PathBuf};

#[test]
fn merging_unresolved_create_quotes_does_not_invent_sol() {
    use sol_shred_sdk::core::{events::*, merger, pumpfun_fee_enrich};
    let mint = Pubkey::new_unique();
    let unknown = || {
        DexEvent::PumpFunCreate(PumpFunCreateTokenEvent {
            mint,
            ix_name: "create_v2".into(),
            ..Default::default()
        })
    };
    for log_preferred in [false, true] {
        let mut base = unknown();
        if log_preferred {
            merger::merge_grpc_instruction_into_log(&mut base, unknown());
        } else {
            merger::merge_events(&mut base, unknown());
        }
        let DexEvent::PumpFunCreate(c) = base else {
            panic!("create")
        };
        assert_eq!(c.quote_mint, Pubkey::default());
    }
    let mut events = vec![
        unknown(),
        DexEvent::PumpFunCreateV2(PumpFunCreateV2TokenEvent {
            mint,
            ..Default::default()
        }),
    ];
    pumpfun_fee_enrich::enrich_create_v2_from_create_events(&mut events);
    let DexEvent::PumpFunCreateV2(c) = &events[1] else {
        panic!("v2")
    };
    assert_eq!(c.quote_mint, Pubkey::default());
}

#[test]
fn create_v2_quote_tail_validation_and_filler_parity() {
    use sol_shred_sdk::core::{account_fillers::pumpfun, events::*};
    let layout: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_create_layout.json")).unwrap();
    let v2 = layout["instructions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "create_v2")
        .unwrap();
    let fixed = v2["accounts"].as_array().unwrap().len();
    let mut data: Vec<u8> = v2["discriminator"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect();
    for value in ["name", "SYM", "uri"] {
        data.extend((value.len() as u32).to_le_bytes());
        data.extend(value.as_bytes());
    }
    data.extend([42; 32]);
    data.extend([0, 0]);
    let quote: Pubkey = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
        .parse()
        .unwrap();
    let token: Pubkey = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
        .parse()
        .unwrap();
    let vault = Pubkey::new_unique();
    let parse = |accounts: &[Pubkey]| {
        let event = sol_shred_sdk::instr::pump::parse_instruction(
            &data,
            accounts,
            Signature::default(),
            1,
            0,
            None,
            0,
        )
        .unwrap();
        let DexEvent::PumpFunCreate(c) = event else {
            panic!("create")
        };
        c
    };
    for extra in [0, 1, 5] {
        let mut accounts = vec![Pubkey::new_unique(); fixed];
        accounts.extend([quote, vault, token]);
        accounts.extend(vec![Pubkey::new_unique(); extra]);
        let c = parse(&accounts);
        assert_eq!(
            (c.quote_mint, c.quote_vault, c.quote_token_program),
            (quote, vault, token)
        );
        let get = |i| accounts.get(i).copied().unwrap_or_default();
        let mut canonical = PumpFunCreateTokenEvent::default();
        pumpfun::fill_create_accounts_from_v2(&mut canonical, &get);
        let mut v2 = PumpFunCreateV2TokenEvent::default();
        pumpfun::fill_create_v2_accounts(&mut v2, &get);
        assert_eq!(
            (
                canonical.quote_mint,
                canonical.quote_vault,
                canonical.quote_token_program
            ),
            (quote, vault, token)
        );
        assert_eq!(
            (v2.quote_mint, v2.quote_vault, v2.quote_token_program),
            (quote, vault, token)
        );
    }
    let mut valid = vec![Pubkey::new_unique(); fixed];
    valid.extend([quote, vault, token]);
    for missing in [fixed, fixed + 1, fixed + 2] {
        let mut accounts = valid.clone();
        accounts[missing] = Pubkey::default();
        let c = parse(&accounts);
        assert_eq!(
            (c.quote_mint, c.quote_vault, c.quote_token_program),
            (Pubkey::default(), Pubkey::default(), Pubkey::default())
        );
    }
    for length in [fixed + 1, fixed + 2] {
        assert_eq!(
            parse(&valid[..length]).quote_mint,
            Pubkey::default(),
            "partial tail is unknown"
        );
    }
    valid[fixed + 2] = Pubkey::new_unique();
    assert_eq!(
        parse(&valid).quote_mint,
        Pubkey::default(),
        "not a token program"
    );
    assert_eq!(
        parse(&valid[..fixed]).quote_mint.to_string(),
        "So11111111111111111111111111111111111111111",
        "no quote tail is native SOL"
    );
}

#[test]
fn mainnet_create_logs_match_independent_idl_wire_decode() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_create_rpc_manifest.json")).unwrap();
    for sample in manifest
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["successful"] == true)
    {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pumpfun_create")
            .join(sample["file"].as_str().unwrap());
        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let expected = sample["create_log_fields"].as_object().unwrap();
        let mut found = false;
        for log in raw["meta"]["logMessages"].as_array().unwrap() {
            let Some(DexEvent::PumpFunCreate(c)) = sol_shred_sdk::logs::pump::parse_log(
                log.as_str().unwrap(),
                Signature::default(),
                sample["slot"].as_u64().unwrap(),
                0,
                None,
                0,
                false,
            ) else {
                continue;
            };
            let actual = serde_json::to_value(c).unwrap();
            for (name, value) in expected {
                let expected = if name == "quote_mint"
                    && value == "11111111111111111111111111111111"
                {
                    serde_json::Value::String("So11111111111111111111111111111111111111111".into())
                } else {
                    value.clone()
                };
                let expected = if actual[name].is_array() && expected.is_string() {
                    serde_json::to_value(expected.as_str().unwrap().parse::<Pubkey>().unwrap())
                        .unwrap()
                } else {
                    expected
                };
                assert_eq!(
                    actual[name], expected,
                    "{} field {name}",
                    sample["signature"]
                );
            }
            found = true;
        }
        assert!(found, "mainnet CreateEvent log");
    }
}

#[test]
fn mainnet_create_account_regressions() {
    let directory = std::env::var_os("PUMPFUN_CREATE_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pumpfun_create")
        });
    let layout: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_create_layout.json")).unwrap();
    let mut successful = 0;
    let mut explicit_quotes = 0;
    for file in fs::read_dir(directory).unwrap() {
        let path = file.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if !raw["meta"]["err"].is_null() {
            continue;
        }
        successful += 1;
        let transaction: sol_shred_sdk::transaction::VersionedTransaction =
            wincode::deserialize(&support::wire(&raw)).unwrap();
        let loaded = |field: &str| -> Vec<Pubkey> {
            raw["meta"]["loadedAddresses"][field]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().parse().unwrap())
                .collect()
        };
        let mut events = vec![];
        parse_transaction_dex_events_with_loaded_addresses(
            &transaction,
            &loaded("writable"),
            &loaded("readonly"),
            transaction.signatures[0],
            raw["slot"].as_u64().unwrap(),
            0,
            0,
            None,
            &mut events,
        )
        .unwrap();
        let creates: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                DexEvent::PumpFunCreate(c) => Some(c),
                _ => None,
            })
            .collect();
        assert!(!creates.is_empty(), "{}", path.display());
        let mut keys: Vec<_> = raw["transaction"]["message"]["accountKeys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        for field in ["writable", "readonly"] {
            keys.extend(
                raw["meta"]["loadedAddresses"][field]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|k| k.as_str().unwrap()),
            );
        }
        for c in creates {
            let ix = raw["transaction"]["message"]["instructions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|ix| {
                    let data = bs58::decode(ix["data"].as_str().unwrap())
                        .into_vec()
                        .unwrap();
                    keys[ix["programIdIndex"].as_u64().unwrap() as usize]
                        == "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"
                        && (data.starts_with(&[24, 30, 200, 40, 5, 28, 7, 119])
                            || data.starts_with(&[214, 144, 76, 236, 95, 139, 49, 180]))
                        && keys[ix["accounts"][0].as_u64().unwrap() as usize] == c.mint.to_string()
                })
                .expect("matching create instruction");
            let data = bs58::decode(ix["data"].as_str().unwrap())
                .into_vec()
                .unwrap();
            let v2 = data.starts_with(&[214, 144, 76, 236, 95, 139, 49, 180]);
            let definition = layout["instructions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["name"] == if v2 { "create_v2" } else { "create" })
                .unwrap();
            let serialized = serde_json::to_value(c).unwrap();
            let instruction_accounts: Vec<Pubkey> = ix["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| keys[n.as_u64().unwrap() as usize].parse().unwrap())
                .collect();
            // Expectations use account names from the official IDL snapshot, not SDK indices.
            for (index, name) in definition["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let name = name.as_str().unwrap();
                if let Some(actual) = serialized
                    .get(name)
                    .filter(|v| v.as_array().is_some_and(|a| a.len() == 32))
                {
                    assert_eq!(
                        *actual,
                        serde_json::to_value(instruction_accounts[index]).unwrap(),
                        "{} field {name}",
                        path.display()
                    );
                }
            }
            let parsed = sol_shred_sdk::instr::pump::parse_instruction(
                &data,
                &instruction_accounts,
                transaction.signatures[0],
                raw["slot"].as_u64().unwrap(),
                0,
                None,
                0,
            )
            .unwrap();
            let DexEvent::PumpFunCreate(direct) = parsed else {
                panic!("create")
            };
            let quote_start = definition["accounts"].as_array().unwrap().len();
            let expected_quote = if v2 && instruction_accounts.len() >= quote_start + 3 {
                explicit_quotes += 1;
                assert_eq!(c.quote_vault, instruction_accounts[quote_start + 1]);
                assert_eq!(c.quote_token_program, instruction_accounts[quote_start + 2]);
                instruction_accounts[quote_start]
            } else {
                assert_eq!(c.quote_vault, Pubkey::default());
                "So11111111111111111111111111111111111111111"
                    .parse()
                    .unwrap()
            };
            let canonical_sol = |mint: Pubkey| {
                if mint.to_string() == "So11111111111111111111111111111111111111112" {
                    "So11111111111111111111111111111111111111111"
                        .parse()
                        .unwrap()
                } else {
                    mint
                }
            };
            assert_eq!(
                canonical_sol(c.quote_mint),
                canonical_sol(expected_quote),
                "{}",
                path.display()
            );
            assert_eq!(
                (
                    canonical_sol(direct.quote_mint),
                    direct.quote_vault,
                    direct.quote_token_program
                ),
                (
                    canonical_sol(c.quote_mint),
                    c.quote_vault,
                    c.quote_token_program
                ),
                "RPC/shred core parity"
            );
        }
    }
    assert!(successful >= 2);
    assert!(
        explicit_quotes >= 2,
        "successful mainnet WSOL and USDC quotes"
    );
}

#[test]
fn historical_create_log_exact_layout() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut data = vec![27, 114, 169, 77, 222, 235, 99, 118];
    for value in ["name", "SYM", "uri"] {
        data.extend((value.len() as u32).to_le_bytes());
        data.extend(value.as_bytes());
    }
    for seed in [1, 2, 3] {
        data.extend([seed; 32]);
    }
    let parse = |bytes: &[u8]| {
        sol_shred_sdk::logs::pump::parse_log(
            &format!("Program data: {}", STANDARD.encode(bytes)),
            Signature::default(),
            1,
            0,
            None,
            0,
            false,
        )
    };
    let event = parse(&data).expect("historical event");
    match event {
        DexEvent::PumpFunCreate(c) => {
            assert_eq!(c.mint, Pubkey::new_from_array([1; 32]));
            assert_eq!(c.creator, Pubkey::default());
            assert_eq!(c.timestamp, 0);
        }
        _ => panic!("create"),
    }
    let mut disc = [0u8; 16];
    disc[..8].copy_from_slice(&data[..8]);
    disc[8..].copy_from_slice(&[155, 167, 108, 32, 122, 76, 173, 64]);
    assert!(matches!(
        sol_shred_sdk::instr::pump_inner::parse_pumpfun_inner_instruction(
            &disc,
            &data[8..],
            Default::default(),
            false
        ),
        Some(DexEvent::PumpFunCreate(_))
    ));
    assert!(matches!(
        sol_shred_sdk::logs::pump::parse_create_from_data(&data[8..], Default::default()),
        Some(DexEvent::PumpFunCreate(_))
    ));
    assert!(parse(&data[..data.len() - 1]).is_none());
    data.push(0);
    assert!(parse(&data).is_none());
}
