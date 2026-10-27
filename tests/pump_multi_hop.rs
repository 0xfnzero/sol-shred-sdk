//! Real version-1 wire transaction; no logs, balances or RPC enter Shred parsing.
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::{
    common::{logs_data::DexInstruction, logs_filters::LogFilter},
    core::events::{DexEvent},
    grpc::{EventType, EventTypeFilter},
    transaction::VersionedTransaction,
};

#[test]
fn official_multi_hop_emits_the_curve_across_shred_entry_points() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_multi_hop_20261025.json")).unwrap();
    let case = &fixture["cases"][0];
    let rpc = &case["wire_rpc"];
    let signed: VersionedTransaction = wincode::deserialize_exact(
        &STANDARD
            .decode(rpc["transaction"][0].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    signed.verify_and_hash_message().unwrap();
    assert_eq!(
        signed.signatures[0].to_string(),
        case["signature"].as_str().unwrap()
    );
    let filter = EventTypeFilter::include_only(vec![EventType::PumpFunTrade]);
    let mut events = Vec::new();
    sol_shred_sdk::shredstream::parse_transaction_dex_events_with_filter(
        &signed,
        signed.signatures[0],
        rpc["slot"].as_u64().unwrap(),
        77,
        123456,
        Some(&filter),
        &mut events,
    );
    assert_eq!(events.len(), 1);
    let DexEvent::PumpFunTrade(event) = &events[0] else {
        panic!("missing curve")
    };
    assert_eq!(event.ix_name, "multi_hop_swap");
    assert_eq!(
        event.mint.to_string(),
        "Aijxkeq5L9iaDYZxfaXUhnHQj4cYeo8um6a3t5vzpump"
    );
    assert_eq!(
        event.quote_mint.to_string(),
        "6Vk3fAQWYEot6JbQb8Nx8j5wZeDnMhzk8FajjHhZpump"
    );
    assert!(event.is_buy && event.is_created_buy);
    assert_eq!(
        (event.sol_amount, event.token_amount, event.quote_amount),
        (0, 0, 0)
    );
    assert_eq!(
        (event.metadata.tx_index, event.metadata.grpc_recv_us),
        (77, 123456)
    );
    let source = event.third_program.as_ref().unwrap();
    assert_eq!(source.instruction_index, 5);
    assert_eq!(
        source.account_indices,
        signed.message.instructions()[5].accounts
    );
    assert_eq!(
        source.instruction_data,
        signed.message.instructions()[5].data
    );
    assert_eq!(source.accounts.len(), 26);

    let legacy = LogFilter::parse_compiled_instruction(&signed, None).unwrap();
    let trades: Vec<_> = legacy
        .iter()
        .filter_map(|e| match e {
            DexInstruction::PumpFunTrade(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].mint, event.mint);
    assert_eq!(trades[0].ix_name, event.ix_name);
    assert_eq!(trades[0].third_program, event.third_program);

    use sol_shred_sdk::shredstream::*;
    let response = SubscribeTransactionsResponse {
        transaction: Some(SubscribeUpdateTransaction {
            slot: rpc["slot"].as_u64().unwrap(),
            transaction: Some(Transaction {
                signatures: signed
                    .signatures
                    .iter()
                    .map(|s| s.as_ref().to_vec())
                    .collect(),
                message: Some(Message {
                    account_keys: signed
                        .message
                        .static_account_keys()
                        .iter()
                        .map(|key| key.to_bytes().to_vec())
                        .collect(),
                    instructions: signed
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
    let proto = LogFilter::parse_compiled_instruction_shreder(&response).unwrap();
    let trades: Vec<_> = proto
        .iter()
        .filter_map(|e| match e {
            DexInstruction::PumpFunTrade(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].mint, event.mint);
    assert_eq!(trades[0].ix_name, event.ix_name);
    assert_eq!(trades[0].third_program, event.third_program);
    assert_eq!(trades[0].metadata.slot, event.metadata.slot);
}
