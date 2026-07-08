pub mod program_ids;
pub mod shared;
pub mod shredstream;
pub mod types;

pub use types::{event_type_from_dex_event, EventType, EventTypeFilter, Protocol};

use std::sync::Arc;

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
use once_cell::sync::Lazy;
use std::num::NonZeroUsize;
use std::sync::Mutex;

const CHANNEL_SIZE: usize = 1000;

// 只保留最近10000个tx的签名
static PROCESSED_TXS: Lazy<Mutex<LruCache<String, ()>>> =
    Lazy::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(10_000).unwrap())));

use lazy_static::lazy_static; // 添加这一行
use tokio::sync::broadcast; // 添加这一行

lazy_static! {
    pub static ref WS_SENDER: broadcast::Sender<String> = {
        let (tx, _rx) = broadcast::channel(100);
        tx
    };
}

pub mod ws_server;

#[deprecated(
    note = "Jito-style ShredStream gRPC is not a supported ingestion path; use shredstream::ShredStreamClient for raw UDP shreds"
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
                        if let Ok(entries) = bincode::deserialize::<Vec<Entry>>(&msg.entries) {
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
                |event| callback(event),
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
        // let versioned_tx = match &response.transaction {
        //     Some(tx) => tx,
        //     None => return Ok(()),  // 改为 Ok(())
        // };

        // let tx = match &versioned_tx.transaction {
        //     Some(t) => t,
        //     None => return Ok(()),  // 改为 Ok(())
        // };
        let slot = 0;

        // 1. 提取 signature
        // let tx_signature = bs58::encode(&tx_info.signature).into_string();
        let tx_signature = bs58::encode(
            &tx_info
                .transaction
                .as_ref()
                .unwrap()
                .transaction
                .as_ref()
                .unwrap()
                .signatures[0],
        )
        .into_string();

        // 2. 去重：如果已处理过该tx，直接返回
        {
            let mut cache = PROCESSED_TXS.lock().unwrap();
            if cache.put(tx_signature.clone(), ()).is_some() {
                // 已经处理过，直接跳过
                return Ok(());
            }
        } // 释放锁

        let mut token_info: Option<CreateTokenInfo> = None;
        let mut dev_trade_info: Option<TradeInfo> = None;
        let mut bonk_token_info: Option<BonkCreateTokenInfo> = None;
        let mut bonk_trade_info: Option<TradeRequest> = None;
        let mut total_sol_amount = 0u64;
        let mut total_token_amount = 0u64;

        let instructions = LogFilter::parse_compiled_instruction_shreder(&tx_info).unwrap();

        for instruction in instructions {
            match instruction {
                DexInstruction::CreateToken(mut token) => {
                    let (limit, price, fee_merchant, fee) =
                        LogFilter::parse_tip_info_shreder(&tx_info);
                    token.slot = slot;
                    token.unit_limit = limit.unwrap_or(0);
                    token.unit_price = price.unwrap_or(0);
                    token.fee_merchant = fee_merchant.unwrap_or_default();
                    token.fee = fee.unwrap_or(0);
                    token_info = Some(token);
                }
                DexInstruction::BonkCreateToken(mut token) => {
                    let (limit, price, fee_merchant, fee) =
                        LogFilter::parse_tip_info_shreder(&tx_info);
                    token.unit_limit = limit.unwrap_or(0);
                    token.unit_price = price.unwrap_or(0);
                    token.fee_merchant = fee_merchant.unwrap_or_default();
                    token.fee = fee.unwrap_or(0);
                    bonk_token_info = Some(token);
                }
                DexInstruction::BonkTrade(trade_request) => {
                    bonk_trade_info = Some(trade_request);
                }
                DexInstruction::UserTrade(mut trade_info) => {
                    trade_info.slot = slot;
                    if token_info.is_some() {
                        total_sol_amount += trade_info.sol_amount;
                        total_token_amount += trade_info.token_amount;
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
        if dev_trade_info.is_some() {
            if let Some(ref mut trade) = &mut dev_trade_info {
                trade.sol_amount = total_sol_amount;
                trade.token_amount = total_token_amount;
            }
        }

        // 用if-else替代match
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
