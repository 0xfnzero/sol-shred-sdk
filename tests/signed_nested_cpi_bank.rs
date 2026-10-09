//! Actual SBF-wrapper bank evidence, with independently verified signed wire.
#[path = "../examples/common/mod.rs"]
mod common;
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::transaction::VersionedTransaction;

#[test]
fn successful_nested_swap_and_failed_parent_have_distinct_settlement() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/signed_nested_cpi_20261009.json")).unwrap();
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 3);
    for case in corpus["cases"].as_array().unwrap() {
        let tx: VersionedTransaction =
            wincode::deserialize_exact(&STANDARD.decode(case["wire"].as_str().unwrap()).unwrap())
                .unwrap();
        tx.sanitize().unwrap();
        tx.verify_and_hash_message().unwrap();
        let mut rpc = case["rpc"].clone();
        rpc["transaction"] = serde_json::json!([case["wire"], "base64"]);
        let (events, route, _) = common::validate(&rpc).unwrap();
        assert_eq!(route.signature, tx.signatures[0]);
        assert_eq!(route.legs.len(), 1);
        let succeeded = case["succeeded"].as_bool().unwrap();
        assert_eq!(route.succeeded, succeeded);
        let leg = &route.legs[0];
        assert_eq!(leg.position.outer_index, 1);
        assert_eq!(
            leg.position.inner_index,
            Some(if succeeded { 0 } else { 1 })
        );
        assert_eq!(
            leg.position.stack_height,
            Some(if succeeded { 2 } else { 3 })
        );
        assert!(
            route.transfers.len() >= 2,
            "rolled-back transfer attempts remain instruction evidence"
        );
        if succeeded {
            assert_eq!(leg.actual_input_amount, Some(10001));
            assert_eq!(leg.actual_output_amount, Some(468207));
        } else {
            assert_eq!(events, 0);
            assert_eq!(leg.actual_input_amount, None);
            assert_eq!(leg.actual_output_amount, None);
            assert!(case["token_deltas"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v.as_i64() == Some(0)));
            assert!(rpc["meta"]["logMessages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v
                    .as_str()
                    .is_some_and(|s| s.contains("CPMMoo8L") && s.ends_with(" success"))));
        }
    }
}
