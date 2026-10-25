//! Offline raw transaction replay. --multi-hop selects the official PumpSwap
//! route; --create-alt selects three reported create_v2 + buy transactions.
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::{transaction::VersionedTransaction, Pubkey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--create-alt") {
        use sol_shred_sdk::grpc::{EventType, EventTypeFilter};
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/pump_create_alt_20261026.json"
        ))?;
        let filter =
            EventTypeFilter::include_only(vec![EventType::PumpFunCreate, EventType::PumpFunTrade]);
        for case in fixture["cases"].as_array().ok_or("missing cases")? {
            let rpc = &case["wire_rpc"];
            let tx: VersionedTransaction = wincode::deserialize_exact(
                &STANDARD.decode(
                    rpc["transaction"][0]
                        .as_str()
                        .ok_or("missing wire transaction")?,
                )?,
            )?;
            tx.verify_and_hash_message()?;
            let mut events = Vec::new();
            // Only the signed transaction enters the parser: no ALT snapshot or RPC.
            sol_shred_sdk::shredstream::pump_ix::parse_transaction_dex_events_with_filter(
                &tx,
                tx.signatures[0],
                rpc["slot"].as_u64().ok_or("missing slot")?,
                888,
                123456,
                Some(&filter),
                &mut events,
            );
            println!(
                "{}",
                serde_json::json!({"mode":"shred_static_pump", "signature":case["signature"], "events":events})
            );
        }
        return Ok(());
    }
    let f: serde_json::Value = if std::env::args().nth(1).as_deref() == Some("--multi-hop") {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/pump_multi_hop_20261025.json"
        ))?;
        let case = &fixture["cases"][0];
        serde_json::json!({
            "signature": case["signature"], "slot": case["wire_rpc"]["slot"],
            "wire": case["wire_rpc"]["transaction"][0],
            "loaded_writable": [], "loaded_readonly": []
        })
    } else {
        serde_json::from_str(include_str!("fixtures/pump_raw_20261019.json"))?
    };
    let tx: VersionedTransaction =
        wincode::deserialize_exact(&STANDARD.decode(f["wire"].as_str().unwrap())?)?;
    assert_eq!(
        tx.signatures[0].to_string(),
        f["signature"].as_str().unwrap()
    );
    let keys = |name: &str| -> Vec<Pubkey> {
        f[name]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap().parse().unwrap())
            .collect()
    };
    let mut events = vec![];
    sol_shred_sdk::shredstream::parse_transaction_dex_events_with_loaded_addresses(
        &tx,
        &keys("loaded_writable"),
        &keys("loaded_readonly"),
        tx.signatures[0],
        f["slot"].as_u64().unwrap(),
        0,
        0,
        None,
        &mut events,
    )?;
    assert!(events.iter().any(|e| e.third_program().is_some()));
    println!("{}", serde_json::to_string_pretty(&events)?);
    Ok(())
}
