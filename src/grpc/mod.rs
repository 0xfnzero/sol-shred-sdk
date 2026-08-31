pub mod program_ids;
pub mod shared;
pub mod shredstream;
pub mod types;

pub use types::{event_type_from_dex_event, EventType, EventTypeFilter, Protocol};

use std::sync::{Arc, LazyLock as Lazy, Mutex};

use futures::{channel::mpsc, StreamExt};
use solana_entry::entry::Entry;
use tonic::transport::Channel;

use log::error;
use solana_sdk::transaction::VersionedTransaction;

pub type AnyResult<T> = anyhow::Result<T>;

use crate::common::logs_data::{
    BonkCreateTokenInfo, CreateTokenInfo, DexInstruction, TradeInfo, TradeRequest,
};
use crate::common::logs_events::PumpfunEvent;
use crate::common::logs_filters::LogFilter;
use crate::grpc::shredstream::shredstream_proxy_client::ShredstreamProxyClient;
use crate::grpc::shredstream::SubscribeEntriesRequest;
use crate::parser::{PumpfunEventParser, PumpfunParserConfig};
use solana_sdk::pubkey::Pubkey;
// use jetstream_protos::jetstream::SubscribeUpdateTransactionInfo;
use crate::shredstream::SubscribeTransactionsResponse;

use lru::LruCache;
use std::num::NonZeroUsize;

const CHANNEL_SIZE: usize = 1000;

// 只保留最近10000个tx的签名
static PROCESSED_TXS: Lazy<Mutex<LruCache<[u8; 64], ()>>> = Lazy::new(|| {
    Mutex::new(LruCache::new(
        NonZeroUsize::new(10_000).unwrap_or(NonZeroUsize::MIN),
    ))
});

use tokio::sync::broadcast;

pub static WS_SENDER: Lazy<broadcast::Sender<String>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(100);
    tx
});

pub mod ws_server;

#[deprecated(
    note = "legacy PumpFun-only wrapper; use shredstream::ShredStreamClient with ShredDecodeMode::JitoGrpc for unified parsing"
)]
pub struct ShredStreamGrpc {
    shredstream_client: Arc<ShredstreamProxyClient<Channel>>,
}

struct TransactionWithSlot {
    transaction: VersionedTransaction,
    slot: u64,
}

#[allow(deprecated)]
impl ShredStreamGrpc {
    pub async fn new(endpoint: String) -> AnyResult<Self> {
        let shredstream_client = ShredstreamProxyClient::connect(endpoint.clone()).await?;
        Ok(Self {
            shredstream_client: Arc::new(shredstream_client),
        })
    }

    pub async fn shredstream_subscribe<F>(
        &self,
        callback: F,
        bot_wallet: Option<Pubkey>,
    ) -> AnyResult<()>
    where
        F: Fn(PumpfunEvent) + Send + Sync + 'static,
    {
        let request = tonic::Request::new(SubscribeEntriesRequest {});
        let mut client = (*self.shredstream_client).clone();
        let mut stream = client.subscribe_entries(request).await?.into_inner();
        let (mut tx, mut rx) = mpsc::channel::<TransactionWithSlot>(CHANNEL_SIZE);
        let callback = Box::new(callback);
        tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                match message {
                    Ok(msg) => {
                        if let Ok(entries) = wincode::deserialize_exact::<Vec<Entry>>(&msg.entries)
                        {
                            for entry in entries {
                                for transaction in entry.transactions {
                                    let _ = tx.try_send(TransactionWithSlot {
                                        transaction,
                                        slot: msg.slot,
                                    });
                                }
                            }
                        }
                    }
                    Err(error) => {
                        error!("Stream error: {error:?}");
                        break;
                    }
                }
            }
        });

        let mut parser =
            PumpfunEventParser::new(PumpfunParserConfig::default().with_bot_wallet(bot_wallet));
        while let Some(transaction_with_slot) = rx.next().await {
            if let Err(e) = parser.process_transaction(
                &transaction_with_slot.transaction,
                transaction_with_slot.slot,
                &callback,
            ) {
                error!("Error processing transaction: {:?}", e);
            }
        }

        Ok(())
    }

    pub async fn process_pumpfun_transaction_shreder<F>(
        tx_info: &SubscribeTransactionsResponse,
        callback: &F,
    ) -> AnyResult<()>
    where
        F: Fn(PumpfunEvent) + Send + Sync,
    {
        let transaction_update = tx_info
            .transaction
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing transaction update"))?;
        let transaction = transaction_update
            .transaction
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing transaction"))?;
        let slot = transaction_update.slot;
        let tx_signature: [u8; 64] = transaction
            .signatures
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing transaction signature"))?
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("transaction signature must be exactly 64 bytes"))?;

        {
            let mut cache = PROCESSED_TXS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if cache.put(tx_signature, ()).is_some() {
                return Ok(());
            }
        }

        let mut token_info: Option<CreateTokenInfo> = None;
        let mut dev_trade_info: Option<TradeInfo> = None;
        let mut bonk_token_info: Option<BonkCreateTokenInfo> = None;
        let mut bonk_trade_info: Option<TradeRequest> = None;
        let mut total_sol_amount = 0u64;
        let mut total_token_amount = 0u64;

        let instructions = LogFilter::parse_compiled_instruction_shreder(tx_info)?;
        let mut tip_info = None;

        for instruction in instructions {
            match instruction {
                DexInstruction::CreateToken(mut token) => {
                    let (unit_limit, unit_price, fee_merchant, fee) =
                        tip_info.get_or_insert_with(|| LogFilter::parse_tip_info_shreder(tx_info));
                    token.slot = slot;
                    token.unit_limit = (*unit_limit).unwrap_or(0);
                    token.unit_price = (*unit_price).unwrap_or(0);
                    token.fee_merchant = fee_merchant.clone().unwrap_or_default();
                    token.fee = (*fee).unwrap_or(0);
                    token_info = Some(token);
                }
                DexInstruction::BonkCreateToken(mut token) => {
                    let (unit_limit, unit_price, fee_merchant, fee) =
                        tip_info.get_or_insert_with(|| LogFilter::parse_tip_info_shreder(tx_info));
                    token.unit_limit = (*unit_limit).unwrap_or(0);
                    token.unit_price = (*unit_price).unwrap_or(0);
                    token.fee_merchant = fee_merchant.clone().unwrap_or_default();
                    token.fee = (*fee).unwrap_or(0);
                    bonk_token_info = Some(token);
                }
                DexInstruction::BonkTrade(trade_request) => {
                    bonk_trade_info = Some(trade_request);
                }
                DexInstruction::UserTrade(mut trade_info) => {
                    trade_info.slot = slot;
                    if token_info.is_some() {
                        total_sol_amount = total_sol_amount.saturating_add(trade_info.sol_amount);
                        total_token_amount =
                            total_token_amount.saturating_add(trade_info.token_amount);
                        dev_trade_info = Some(trade_info);
                    }
                }
                DexInstruction::BotTrade(mut trade_info) => {
                    trade_info.slot = slot;
                    callback(PumpfunEvent::NewBotTrade(trade_info));
                }
                _ => {}
            }
        }
        if let Some(trade) = &mut dev_trade_info {
            trade.sol_amount = total_sol_amount;
            trade.token_amount = total_token_amount;
        }

        match (token_info, dev_trade_info, bonk_token_info, bonk_trade_info) {
            (Some(token), Some(trade), None, None) => {
                let combined_event = PumpfunEvent::NewToken2 { token, trade };
                callback(combined_event);
            }
            (Some(token), None, None, None) => {
                callback(PumpfunEvent::NewToken(token));
            }
            (None, None, Some(bonk_token), Some(bonk_trade)) => {
                callback(PumpfunEvent::NewBonkToken {
                    token: bonk_token,
                    trade: bonk_trade,
                });
            }
            _ => {}
        }

        Ok(())
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::instr::program_ids::PUMPFUN_PROGRAM_ID;
    use crate::shredstream::{
        CompiledInstruction, Message, SubscribeUpdateTransaction, Transaction,
    };

    fn push_borsh_string(buffer: &mut Vec<u8>, value: &str) {
        buffer.extend_from_slice(&(value.len() as u32).to_le_bytes());
        buffer.extend_from_slice(value.as_bytes());
    }

    fn create_response(signature: Vec<u8>, slot: u64) -> SubscribeTransactionsResponse {
        let mut data = LogFilter::CREATE_TOKEN_IX.to_vec();
        push_borsh_string(&mut data, "Token");
        push_borsh_string(&mut data, "TKN");
        push_borsh_string(&mut data, "https://example.invalid/token.json");
        data.extend_from_slice(Pubkey::new_unique().as_ref());

        let mut account_keys = vec![PUMPFUN_PROGRAM_ID.to_bytes().to_vec()];
        account_keys.extend((0..8).map(|_| Pubkey::new_unique().to_bytes().to_vec()));
        SubscribeTransactionsResponse {
            transaction: Some(SubscribeUpdateTransaction {
                transaction: Some(Transaction {
                    signatures: vec![signature],
                    message: Some(Message {
                        account_keys,
                        instructions: vec![CompiledInstruction {
                            program_id_index: 0,
                            accounts: (1..=8).collect(),
                            data,
                        }],
                        ..Message::default()
                    }),
                }),
                slot,
            }),
            ..SubscribeTransactionsResponse::default()
        }
    }

    #[test]
    fn protobuf_processor_preserves_transaction_slot() {
        let response = create_response(vec![41; 64], 987_654);
        let events = Mutex::new(Vec::new());
        let callback = |event| {
            events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(event);
        };
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(ShredStreamGrpc::process_pumpfun_transaction_shreder(
                &response, &callback,
            ))
            .expect("valid protobuf transaction");

        let events = events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(matches!(
            events.as_slice(),
            [PumpfunEvent::NewToken(token)] if token.slot == 987_654
        ));
    }

    #[test]
    fn protobuf_processor_rejects_invalid_signature_length() {
        let response = create_response(vec![1; 63], 1);
        let result = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(ShredStreamGrpc::process_pumpfun_transaction_shreder(
                &response,
                &|_| {},
            ));
        assert!(result.is_err());
    }
}
