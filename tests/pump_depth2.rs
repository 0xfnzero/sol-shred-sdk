use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::{parse_transaction_dex_events_with_loaded_addresses, DexEvent, Pubkey};
#[test]
fn signed_depth2_shred_and_logs() {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_depth2_20261014.json")).unwrap();
    for c in f["cases"].as_array().unwrap() {
        let tx: sol_shred_sdk::transaction::VersionedTransaction = wincode::deserialize(
            &STANDARD
                .decode(c["wire_rpc"]["transaction"][0].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        let loaded = |field: &str| -> Vec<Pubkey> {
            c["raw"]["meta"]["loadedAddresses"][field]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_str().unwrap().parse().unwrap())
                .collect()
        };
        let mut events = vec![];
        parse_transaction_dex_events_with_loaded_addresses(
            &tx,
            &loaded("writable"),
            &loaded("readonly"),
            tx.signatures[0],
            100,
            0,
            0,
            None,
            &mut events,
        )
        .unwrap();
        let creates: Vec<_> = events
            .iter()
            .filter_map(|e| {
                if let DexEvent::PumpFunCreate(e) = e {
                    Some(e)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(creates.len(), 1, "{}", c["name"]);
        let e = creates[0];
        assert_eq!(e.mint.to_string(), c["expected"]["mint"].as_str().unwrap());
        assert_eq!(
            e.quote_mint.to_string(),
            c["expected"]["quote_mint"].as_str().unwrap()
        );
        let mut logged = 0;
        for line in c["raw"]["meta"]["logMessages"].as_array().unwrap() {
            if let Some(DexEvent::PumpFunCreate(e)) = sol_shred_sdk::logs::pump::parse_log(
                line.as_str().unwrap(),
                tx.signatures[0],
                100,
                0,
                None,
                0,
                false,
            ) {
                logged += 1;
                assert_eq!(e.depth, 2);
                assert_eq!(
                    e.virtual_quote_reserves.to_string(),
                    c["expected"]["virtual_quote_reserves"].as_str().unwrap()
                );
            }
        }
        assert_eq!(logged, 1);
    }
}
