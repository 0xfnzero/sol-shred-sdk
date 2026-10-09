//! Actual Python SDK-built signed two-leg CPMM bank execution; metadata is a projection.
#[path = "../examples/common/mod.rs"]
mod common;
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::transaction::VersionedTransaction;

#[test]
fn signed_same_pool_multileg_settlement_is_per_invocation_and_rollback_is_global() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/signed_multileg_cpmm_20261009.json")).unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        let tx: VersionedTransaction =
            wincode::deserialize_exact(&STANDARD.decode(case["wire"].as_str().unwrap()).unwrap())
                .unwrap();
        tx.sanitize().unwrap();
        tx.verify_and_hash_message().unwrap();
        let mut rpc = case["rpc"].clone();
        rpc["transaction"] = serde_json::json!([case["wire"], "base64"]);
        let (_, route, _) = common::validate(&rpc).unwrap();
        assert_eq!(route.signature, tx.signatures[0]);
        assert_eq!(route.legs.len(), 2);
        assert_eq!(route.legs[0].pool, route.legs[1].pool);
        assert_eq!(route.succeeded, case["succeeded"].as_bool().unwrap());
        for (index, leg) in route.legs.iter().enumerate() {
            assert_eq!(leg.position.outer_index as usize, index + 1);
            if !route.succeeded {
                assert_eq!(leg.actual_input_amount, None);
                assert_eq!(leg.actual_output_amount, None);
            } else {
                assert_eq!(
                    leg.actual_input_amount,
                    case["legs"][index]["gross_input"].as_u64()
                );
                let expected = if case["fee_output_legs"][index].as_bool().unwrap() {
                    None
                } else {
                    case["legs"][index]["net_output"].as_u64()
                };
                assert_eq!(leg.actual_output_amount, expected);
            }
        }
    }
}
