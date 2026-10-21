//! Compatibility entry points must obey the same raw/executed boundary.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use sol_shred_sdk::{
    common::{logs_data::DexInstruction, logs_filters::LogFilter},
    transaction::VersionedTransaction,
    PumpfunEvent, PumpfunEventParser,
};

fn cases() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!("fixtures/pump_execution_20261019.json")).unwrap()
        ["cases"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn legacy_logs_preserve_all_actual_pump_executions() {
    for c in cases() {
        let raw = &c["raw"];
        let logs: Vec<String> = serde_json::from_value(raw["meta"]["logMessages"].clone()).unwrap();
        let expected: Vec<_> = raw["meta"]["innerInstructions"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["instructions"].as_array().unwrap())
            .filter_map(|ix| {
                let data = sol_shred_sdk::bs58::decode(ix["data"].as_str().unwrap())
                    .into_vec()
                    .unwrap();
                if !data.starts_with(&[
                    228, 69, 165, 46, 81, 203, 154, 29, 189, 219, 127, 211, 78, 230, 97, 238,
                ]) {
                    return None;
                }
                Some((
                    u64::from_le_bytes(data[56..64].try_into().unwrap()),
                    data[64] != 0,
                ))
            })
            .collect();
        let parsed = LogFilter::parse_instruction(&logs, None).unwrap();
        let actual: Vec<_> = parsed
            .iter()
            .filter_map(|e| match e {
                DexInstruction::UserTrade(t) | DexInstruction::BotTrade(t) => {
                    Some((t.token_amount, t.is_buy))
                }
                _ => None,
            })
            .collect();
        assert_eq!(actual, expected, "{}", c["name"]);
    }
}

#[test]
fn nested_pump_legs_commit_only_with_successful_router() {
    let c = cases()
        .into_iter()
        .find(|c| c["name"] == "delta_sell")
        .unwrap();
    let event = c["raw"]["meta"]["logMessages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|s| {
            s.strip_prefix("Program data: ")
                .and_then(|s| STANDARD.decode(s).ok())
                .is_some_and(|data| data.starts_with(&[189, 219, 127, 211, 78, 230, 97, 238]))
        })
        .unwrap()
        .to_owned();
    let pump = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
    let router = "11111111111111111111111111111111";
    let mut logs = vec![
        format!("Program {router} invoke [1]"),
        format!("Program {pump} invoke [2]"),
        "Program log: Instruction: MultiHopSwap".into(),
    ];
    for _ in 0..2 {
        logs.extend([
            format!("Program {pump} invoke [3]"),
            event.clone(),
            format!("Program {pump} success"),
        ]);
    }
    logs.extend([
        format!("Program {pump} success"),
        format!("Program {router} success"),
    ]);
    assert_eq!(LogFilter::parse_instruction(&logs, None).unwrap().len(), 2);
    *logs.last_mut().unwrap() = format!("Program {router} failed: custom program error: 0x1");
    assert!(LogFilter::parse_instruction(&logs, None)
        .unwrap()
        .is_empty());
    logs.truncate(logs.len() - 1);
    assert!(LogFilter::parse_instruction(&logs, None)
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[allow(deprecated)]
async fn both_legacy_shred_transports_forward_raw_wrappers() {
    for c in cases()
        .into_iter()
        .filter(|c| c["name"] == "delta_sell" || c["name"] == "rapid_create_buy")
    {
        let tx: VersionedTransaction = wincode::deserialize_exact(
            &STANDARD
                .decode(c["wire_rpc"]["transaction"][0].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        let mut events = vec![];
        PumpfunEventParser::default()
            .process_transaction(&tx, c["raw"]["slot"].as_u64().unwrap(), |e| events.push(e))
            .unwrap();
        let raw: Vec<_> = events
            .iter()
            .filter_map(|e| {
                if let PumpfunEvent::PumpFunTrade(r) = e {
                    r.third_program.as_ref()
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(raw.len(), 1);
        let ix = &tx.message.instructions()[raw[0].instruction_index];
        assert_eq!(raw[0].instruction_data, ix.data);
        assert_eq!(raw[0].account_indices, ix.accounts);
        use sol_shred_sdk::shredstream::*;
        let response = SubscribeTransactionsResponse {
            transaction: Some(SubscribeUpdateTransaction {
                slot: c["raw"]["slot"].as_u64().unwrap(),
                transaction: Some(Transaction {
                    signatures: tx.signatures.iter().map(|s| s.as_ref().to_vec()).collect(),
                    message: Some(Message {
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
                            .map(|ix| CompiledInstruction {
                                program_id_index: ix.program_id_index.into(),
                                accounts: ix.accounts.clone(),
                                data: ix.data.clone(),
                            })
                            .collect(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let received = std::sync::Mutex::new(vec![]);
        sol_shred_sdk::ShredStreamGrpc::process_pumpfun_transaction_shreder(&response, &|e| {
            received.lock().unwrap().push(e)
        })
        .await
        .unwrap();
        let received = received.into_inner().unwrap();
        assert_eq!(
            received
                .iter()
                .filter(|e| matches!(e, PumpfunEvent::PumpFunTrade(_)))
                .count(),
            1
        );
    }
}
