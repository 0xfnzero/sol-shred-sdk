//! Replay independently signed/executed Token-2022 CPMM bank transactions.
//! Local captured-program execution; neither these tests nor signatures imply mainnet funding.
#[path = "../examples/common/mod.rs"]
mod common;
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::transaction::VersionedTransaction;

#[test]
fn rpc_validation_rejects_tampered_signed_wire() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/signed_cpmm_bank_20261009.json")).unwrap();
    let case = &corpus["cases"][0];
    let wire = STANDARD.decode(case["wire"].as_str().unwrap()).unwrap();
    for change_message in [false, true] {
        let mut tx: VersionedTransaction = wincode::deserialize_exact(&wire).unwrap();
        if change_message {
            // Keep a well-formed message but change the signed swap argument.
            match &mut tx.message {
                sol_shred_sdk::message::VersionedMessage::Legacy(m) => {
                    m.instructions.last_mut().unwrap().data[8] ^= 1;
                }
                sol_shred_sdk::message::VersionedMessage::V0(m) => {
                    m.instructions.last_mut().unwrap().data[8] ^= 1;
                }
                _ => panic!("unexpected corpus message version"),
            }
        } else {
            tx.signatures[0] = sol_shred_sdk::signature::Signature::default();
        }
        let mut rpc = case["rpc"].clone();
        rpc["transaction"] = serde_json::json!([
            STANDARD.encode(wincode::serialize(&tx).unwrap()), "base64"
        ]);
        for result in [common::validate(&rpc).map(|_| ()), common::instructions::validate(&rpc).map(|_| ())] {
            let error = result.unwrap_err().to_string();
            assert!(error.contains("invalid transaction signature"), "{error}");
        }
    }
}

#[test]
fn signed_cpmm_bank_gross_fee_net_and_failure_attribution() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/signed_cpmm_bank_20261009.json")).unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        let wire = STANDARD.decode(case["wire"].as_str().unwrap()).unwrap();
        let tx: VersionedTransaction = wincode::deserialize_exact(&wire).unwrap();
        let required = tx.message.header().num_required_signatures as usize;
        assert!(required > 0);
        assert_eq!(tx.signatures.len(), required);
        for (signature, key) in tx.signatures.iter().zip(tx.message.static_account_keys()) {
            assert!(
                signature.verify(key.as_ref(), &tx.message.serialize()),
                "{}",
                case["name"]
            );
        }
        let mut rpc = case["rpc"].clone();
        // The corpus keeps parsed RPC messages for cross-language replay;
        // the raw-wire validator consumes getTransaction's base64 encoding.
        rpc["transaction"] = serde_json::json!([case["wire"], "base64"]);
        let (_, route, _) = common::validate(&rpc).unwrap();
        assert_eq!(route.signature, tx.signatures[0]);
        assert_eq!(route.legs.len(), 1);
        assert_eq!(route.succeeded, case["succeeded"].as_bool().unwrap());
        let leg = &route.legs[0];
        if !route.succeeded {
            assert_eq!(leg.actual_input_amount, None);
            assert_eq!(leg.actual_output_amount, None);
        } else {
            assert_eq!(leg.actual_input_amount, case["gross_input"].as_u64());
            if case["name"] == "fee-output" {
                assert_ne!(case["net_output"], case["vault_debit"]);
                assert_eq!(
                    leg.actual_output_amount, None,
                    "plain Token-2022 CPI does not prove net credit"
                );
            } else {
                assert_eq!(leg.actual_output_amount, case["net_output"].as_u64());
            }
        }
    }
}

#[test]
fn rpc_validation_requires_explicit_execution_status() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!("fixtures/signed_cpmm_bank_20261009.json")).unwrap();
    let case = &corpus["cases"][0];
    let mut rpc = case["rpc"].clone();
    rpc["transaction"] = serde_json::json!([case["wire"], "base64"]);
    rpc["meta"].as_object_mut().unwrap().remove("err");
    assert!(common::validate(&rpc).unwrap_err().to_string().contains("execution status missing"));
    assert!(common::instructions::validate(&rpc).unwrap_err().to_string().contains("execution status missing"));
}
