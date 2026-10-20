//! Offline signed-wire replay. No RPC; ALT keys come from the saved capture.
//! Production consumers load reviewed rules once and resolve ALT from local cache.
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::{
    grpc::{EventType, EventTypeFilter},
    transaction::VersionedTransaction,
    wrapped_pumpfun_parser::{
        parse_wrapped_pumpfun_instruction, restore_instruction, WrappedPumpfunRule,
    },
    DexEvent, Pubkey,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/wrapped_pumpfun_reported.json"))?;
    let rules: Vec<WrappedPumpfunRule> =
        serde_json::from_str(include_str!("fixtures/wrapped_pumpfun_rules.json"))?;
    let mut count = 0;
    for c in fixture.as_array().ok_or("expected cases")? {
        let signed: VersionedTransaction =
            wincode::deserialize_exact(&STANDARD.decode(c["wire"].as_str().unwrap())?)?;
        assert_eq!(
            signed.signatures[0].to_string(),
            c["signature"].as_str().unwrap()
        );
        for (sig, key) in signed
            .signatures
            .iter()
            .zip(signed.message.static_account_keys())
        {
            assert!(sig.verify(key.as_ref(), &signed.message.serialize()));
        }
        let keys = |name: &str| -> Vec<Pubkey> {
            c[name]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_str().unwrap().parse().unwrap())
                .collect()
        };
        let writable = keys("loaded_writable");
        let readonly = keys("loaded_readonly");
        let mut all = signed.message.static_account_keys().to_vec();
        all.extend(&writable);
        all.extend(&readonly);
        let mut events = vec![];
        sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
            &signed,
            &writable,
            &readonly,
            signed.signatures[0],
            c["slot"].as_u64().unwrap(),
            77,
            123456,
            Some(&EventTypeFilter::include_only(vec![EventType::RawInstruction])),
            &mut events,
        )?;
        let mut wrapped = vec![];
        for event in &events {
            let DexEvent::RawInstruction(raw) = event else {
                continue;
            };
            for rule in &rules {
                let Some((data, accounts)) = restore_instruction(raw, rule) else {
                    continue;
                };
                // This oracle is test-only. The parser above sees only outer data.
                let group = c["raw"]["meta"]["innerInstructions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|g| g["index"].as_u64() == Some(raw.instruction_index as u64))
                    .unwrap();
                let ix = group["instructions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|ix| {
                        all[ix["programIdIndex"].as_u64().unwrap() as usize]
                            == sol_shred_sdk::instr::pump::PROGRAM_ID_PUBKEY
                            && bs58::decode(ix["data"].as_str().unwrap())
                                .into_vec()
                                .unwrap()
                                .starts_with(&data[..8])
                    })
                    .unwrap();
                assert_eq!(
                    data.as_slice(),
                    bs58::decode(ix["data"].as_str().unwrap()).into_vec()?
                );
                assert_eq!(
                    accounts,
                    ix["accounts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|i| all[i.as_u64().unwrap() as usize])
                        .collect::<Vec<_>>()
                );
                let event = parse_wrapped_pumpfun_instruction(raw, rule, None).unwrap();
                let DexEvent::PumpFunTrade(t) = &event else {
                    panic!()
                };
                assert!(t.third_program.is_some());
                assert_eq!((t.metadata.tx_index, t.metadata.grpc_recv_us), (77, 123456));
                assert_eq!((t.sol_amount, t.quote_amount, t.token_amount), (0, 0, 0));
                wrapped.push(event);
                count += 1;
                break;
            }
        }
        println!(
            "{}",
            serde_json::json!({"signature": c["signature"], "shred": events, "opt_in_wrapped": wrapped})
        );
    }
    assert_eq!(count, 5);
    Ok(())
}
