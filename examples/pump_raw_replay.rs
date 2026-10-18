//! Current Delta wrapper: preserve raw accounts/data, never infer the internal sell.
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::{core::events::DexEvent, transaction::VersionedTransaction, Pubkey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_raw_20261019.json"))?;
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
    assert!(events
        .iter()
        .any(|e| matches!(e, DexEvent::RawInstruction(_))));
    println!("{}", serde_json::to_string_pretty(&events)?);
    Ok(())
}
