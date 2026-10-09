//! Opt-in optimized measurements of production parser entry points, no networking.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sol_shred_sdk::{
    analyze_yellowstone_transaction_routes, transaction::VersionedTransaction, Pubkey,
};
use std::time::Instant;
use yellowstone_grpc_proto::prelude as pb;

fn measure(name: &str, mut operation: impl FnMut()) -> Value {
    let mut rounds = Vec::new();
    for _ in 0..3 {
        for _ in 0..200 {
            operation();
        }
        let mut samples = Vec::with_capacity(1000);
        for _ in 0..1000 {
            let start = Instant::now();
            operation();
            samples.push(start.elapsed().as_nanos() as f64 / 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        rounds.push(
            json!({"p50":samples[499],"p95":samples[949],"p99":samples[989],
            "maximum":samples[999],"samples_us":samples}),
        );
    }
    json!({"name":name,"unit":"microseconds","warmup_per_round":200,"measured_per_round":1000,"rounds":rounds})
}

#[test]
#[ignore = "explicit offline optimized CPU measurement, no fixed latency SLA"]
fn measure_actual_bank_wire_and_parser_entrypoints() {
    assert!(!cfg!(debug_assertions), "run cargo test --release");
    let corpus: Value = serde_json::from_str(include_str!(
        "fixtures/signed_identical_alt_cpmm_20261009.json"
    ))
    .unwrap();
    let mut paths = Vec::new();
    for case in &corpus["cases"].as_array().unwrap()[..2] {
        let wire = STANDARD.decode(case["wire"].as_str().unwrap()).unwrap();
        let tx: VersionedTransaction = wincode::deserialize_exact(&wire).unwrap();
        tx.sanitize().unwrap();
        tx.verify_and_hash_message().unwrap(); // Cold independent validation, excluded from timing.
        let addresses = |class: &str| -> Vec<Pubkey> {
            case["rpc"]["meta"]["loadedAddresses"][class]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().parse().unwrap())
                .collect()
        };
        let writable = addresses("writable");
        let readonly = addresses("readonly");
        let transaction = pb::Transaction {
            signatures: tx.signatures.iter().map(|s| s.as_ref().to_vec()).collect(),
            message: Some(pb::Message {
                account_keys: tx
                    .message
                    .static_account_keys()
                    .iter()
                    .map(|k| k.to_bytes().to_vec())
                    .collect(),
                instructions: tx
                    .message
                    .instructions()
                    .iter()
                    .map(|ix| pb::CompiledInstruction {
                        program_id_index: ix.program_id_index.into(),
                        accounts: ix.accounts.clone(),
                        data: ix.data.clone(),
                    })
                    .collect(),
                ..Default::default()
            }),
        };
        let mut meta = pb::TransactionStatusMeta {
            loaded_writable_addresses: writable.iter().map(|k| k.to_bytes().to_vec()).collect(),
            loaded_readonly_addresses: readonly.iter().map(|k| k.to_bytes().to_vec()).collect(),
            ..Default::default()
        };
        for group in case["rpc"]["meta"]["innerInstructions"].as_array().unwrap() {
            meta.inner_instructions.push(pb::InnerInstructions {
                index: group["index"].as_u64().unwrap() as u32,
                instructions: group["instructions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|ix| pb::InnerInstruction {
                        program_id_index: ix["programIdIndex"].as_u64().unwrap() as u32,
                        accounts: ix["accounts"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_u64().unwrap() as u8)
                            .collect(),
                        data: bs58::decode(ix["data"].as_str().unwrap())
                            .into_vec()
                            .unwrap(),
                        stack_height: ix["stackHeight"].as_u64().map(|v| v as u32),
                    })
                    .collect(),
            });
        }
        let route = analyze_yellowstone_transaction_routes(&transaction, &meta, &[]);
        assert!(route.succeeded);
        assert_eq!(route.legs.len(), 2);
        assert_eq!(route.legs[0].actual_output_amount, Some(468207));
        assert_eq!(route.legs[1].actual_output_amount, Some(426283));
        let case_name = case["name"].as_str().unwrap();
        paths.push(measure(
            &format!("Shred release exact wire decode {case_name}"),
            || {
                std::hint::black_box(
                    wincode::deserialize_exact::<VersionedTransaction>(&wire).unwrap(),
                );
            },
        ));
        let mut events = Vec::with_capacity(2);
        paths.push(measure(
            &format!("Shred release resolved outer events {case_name}"),
            || {
                events.clear();
                sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
                    &tx,
                    &writable,
                    &readonly,
                    tx.signatures[0],
                    42,
                    0,
                    0,
                    None,
                    &mut events,
                )
                .unwrap();
                std::hint::black_box(&events);
            },
        ));
        assert_eq!(events.len(), 2);
        paths.push(measure(
            &format!("Shred release route + CPI settlement {case_name}"),
            || {
                std::hint::black_box(analyze_yellowstone_transaction_routes(
                    &transaction,
                    &meta,
                    &[],
                ));
            },
        ));
    }
    let result = json!({"paths":paths,"measured_rpc_calls":0,
        "environment":{"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"debug_assertions":cfg!(debug_assertions)},
        "scope":"Actual release SDK library; bank wire/cold verification/metadata conversion outside timing; exact decode, resolved outer events (reused output Vec), and route/CPI settlement measured separately. RPC, shred transport/FEC, bank execution and landing excluded."});
    std::fs::write(
        std::env::var("SDK_LATENCY_OUTPUT").expect("set SDK_LATENCY_OUTPUT"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}
