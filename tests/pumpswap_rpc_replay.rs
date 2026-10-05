#[path = "../examples/common/pumpswap_verify.rs"]
mod verify;

fn samples() -> [(&'static str, &'static str); 7] {
    [
        (
            "sell-v0.json",
            include_str!("fixtures/pumpswap_rpc/sell-v0.json"),
        ),
        (
            "sell-v1.json",
            include_str!("fixtures/pumpswap_rpc/sell-v1.json"),
        ),
        (
            "buy-legacy.json",
            include_str!("fixtures/pumpswap_rpc/buy-legacy.json"),
        ),
        (
            "discovered-0.json",
            include_str!("fixtures/pumpswap_rpc/discovered-0.json"),
        ),
        (
            "discovered-1.json",
            include_str!("fixtures/pumpswap_rpc/discovered-1.json"),
        ),
        (
            "discovered-2.json",
            include_str!("fixtures/pumpswap_rpc/discovered-2.json"),
        ),
        (
            "discovered-3.json",
            include_str!("fixtures/pumpswap_rpc/discovered-3.json"),
        ),
    ]
}

#[test]
fn rpc_mainnet_trades_match_wire_cpi_logs_and_frozen_reserves() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pumpswap_rpc/manifest.json")).unwrap();
    assert_eq!(manifest.as_array().unwrap().len(), samples().len());
    for (file, raw) in samples() {
        let expected = manifest
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["file"] == file)
            .unwrap();
        let hash = sol_shred_sdk::hash::hashv(&[raw.as_bytes()])
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(hash, expected["sha256"].as_str().unwrap());
        let j: serde_json::Value = serde_json::from_str(raw).unwrap();
        let report = verify::verify(&j).unwrap();
        assert_eq!(report.signature, expected["signature"].as_str().unwrap());
        assert_eq!(report.slot, expected["slot"].as_u64().unwrap());
        assert_eq!(report.version, expected["version"]);
        assert_eq!(report.checked_instructions, 1);
        assert_eq!(report.checked_log_cpi_pairs, 1);
        assert_eq!(report.trades.len(), 1);
        let t = &report.trades[0];
        let frozen = &expected["expected_trade"];
        assert_eq!(t.side, frozen["side"].as_str().unwrap());
        assert_eq!(t.base_reserve, frozen["base_reserve"].as_u64().unwrap());
        assert_eq!(
            t.raw_quote_reserve,
            frozen["raw_quote_reserve"].as_u64().unwrap()
        );
        assert_eq!(
            t.virtual_quote_reserve,
            frozen["virtual_quote_reserve"].as_str().unwrap()
        );
        assert_eq!(
            t.effective_quote_reserve,
            frozen["effective_quote_reserve"].as_u64().unwrap()
        );
    }
}

#[test]
fn changed_cpi_payload_is_detected() {
    let mut j: serde_json::Value = serde_json::from_str(samples()[0].1).unwrap();
    let mut changed = false;
    for group in j["meta"]["innerInstructions"].as_array_mut().unwrap() {
        for ix in group["instructions"].as_array_mut().unwrap() {
            let mut data = bs58::decode(ix["data"].as_str().unwrap())
                .into_vec()
                .unwrap();
            if data.starts_with(&sol_shred_sdk::instr::pump_amm_inner::discriminators::SELL) {
                data[16 + 48] ^= 1;
                ix["data"] = bs58::encode(data).into_string().into();
                changed = true;
            }
        }
    }
    assert!(changed);
    assert!(verify::verify(&j)
        .unwrap_err()
        .to_string()
        .contains("no identical CPI"));
}

#[test]
fn missing_or_failed_rpc_execution_is_not_accepted() {
    let original: serde_json::Value = serde_json::from_str(samples()[0].1).unwrap();
    let mut failed = original.clone();
    failed["meta"]["err"] = serde_json::json!({"InstructionError":[0,"Custom"]});
    assert!(verify::verify(&failed).is_err());
    let mut missing = original;
    missing["meta"]["logMessages"] = serde_json::json!([]);
    assert!(verify::verify(&missing)
        .unwrap_err()
        .to_string()
        .contains("missing from logs"));
}

#[test]
fn truncated_supported_cpi_is_an_error() {
    let mut j: serde_json::Value = serde_json::from_str(samples()[0].1).unwrap();
    let mut changed = false;
    for group in j["meta"]["innerInstructions"].as_array_mut().unwrap() {
        for ix in group["instructions"].as_array_mut().unwrap() {
            let mut data = bs58::decode(ix["data"].as_str().unwrap())
                .into_vec()
                .unwrap();
            if data.starts_with(&sol_shred_sdk::instr::pump_amm_inner::discriminators::SELL) {
                data.truncate(16 + 351);
                ix["data"] = bs58::encode(data).into_string().into();
                changed = true;
            }
        }
    }
    assert!(changed);
    assert!(verify::verify(&j)
        .unwrap_err()
        .to_string()
        .contains("supported CPI event rejected"));
}
