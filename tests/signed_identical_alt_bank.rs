//! Actual signed bank transactions: identical intents must retain separate settlement.
#[path = "../examples/common/mod.rs"]
mod common;
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::{transaction::VersionedTransaction, Pubkey};

#[test]
fn identical_swaps_with_two_alt_tables_preserve_invocations_and_global_rollback() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/signed_identical_alt_cpmm_20261009.json"
    ))
    .unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    for case in cases {
        let tx: VersionedTransaction =
            wincode::deserialize_exact(&STANDARD.decode(case["wire"].as_str().unwrap()).unwrap())
                .unwrap();
        tx.sanitize().unwrap();
        tx.verify_and_hash_message().unwrap();
        assert_eq!(tx.signatures.len(), 2);
        assert_eq!(
            tx.message.instructions()[1].data[..16],
            tx.message.instructions()[2].data[..16]
        );
        let tables = case["lookup_tables"].as_array().unwrap();
        let lookups = tx.message.address_table_lookups().unwrap();
        assert_eq!(lookups.len(), tables.len());
        for class in ["writable", "readonly"] {
            let mut resolved = Vec::new();
            for lookup in lookups {
                let table = tables
                    .iter()
                    .find(|t| t["key"].as_str().unwrap() == lookup.account_key.to_string())
                    .unwrap();
                assert!(!lookup.writable_indexes.is_empty() && !lookup.readonly_indexes.is_empty());
                let indexes = if class == "writable" {
                    &lookup.writable_indexes
                } else {
                    &lookup.readonly_indexes
                };
                for &index in indexes {
                    resolved.push(
                        table["addresses"][index as usize]
                            .as_str()
                            .unwrap()
                            .parse::<Pubkey>()
                            .unwrap(),
                    );
                }
            }
            let actual: Vec<Pubkey> = case["rpc"]["meta"]["loadedAddresses"][class]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().parse().unwrap())
                .collect();
            assert_eq!(resolved, actual);
        }
        let mut rpc = case["rpc"].clone();
        rpc["transaction"] = serde_json::json!([case["wire"], "base64"]);
        let (_, route, _) = common::validate(&rpc).unwrap();
        assert_eq!(route.legs.len(), 2);
        assert_eq!(route.legs[0].pool, route.legs[1].pool);
        assert_eq!(route.succeeded, case["succeeded"].as_bool().unwrap());
        for (index, leg) in route.legs.iter().enumerate() {
            assert_eq!(leg.position.outer_index as usize, index + 1);
            if route.succeeded {
                assert_eq!(leg.actual_input_amount, Some(10001));
                assert_eq!(
                    leg.actual_output_amount,
                    case["legs"][index]["net_output"].as_u64()
                );
            } else {
                assert_eq!(leg.actual_input_amount, None);
                assert_eq!(leg.actual_output_amount, None);
            }
        }
        if route.succeeded {
            assert_ne!(
                route.legs[0].actual_output_amount,
                route.legs[1].actual_output_amount
            );
        } else {
            assert_eq!(case["before"], case["after"]);
        }
    }
}
