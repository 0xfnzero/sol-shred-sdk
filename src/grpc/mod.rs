pub mod shredstream;
pub mod shared; 

use std::sync::Arc;

use futures::{channel::mpsc, StreamExt};
use solana_entry::entry::Entry;
use tonic::transport::Channel;

use log::error;
use solana_sdk::transaction::VersionedTransaction;

pub type AnyResult<T> = anyhow::Result<T>;

use solana_sdk::pubkey::Pubkey;
use crate::common::logs_data::{DexInstruction, CreateTokenInfo, TradeInfo, BonkCreateTokenInfo, TradeRequest};
use crate::common::logs_events::PumpfunEvent;
use crate::common::logs_filters::LogFilter;
use crate::grpc::shredstream::shredstream_proxy_client::ShredstreamProxyClient;
use crate::grpc::shredstream::SubscribeEntriesRequest;
// use jetstream_protos::jetstream::SubscribeUpdateTransactionInfo;
use crate::shredstream::SubscribeTransactionsResponse;
use crate::shredstream::CompiledInstruction; // 添加这一行


use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use once_cell::sync::Lazy;

const CHANNEL_SIZE: usize = 1000;


// 只保留最近10000个tx的签名
static PROCESSED_TXS: Lazy<Mutex<LruCache<String, ()>>> = Lazy::new(|| {
    Mutex::new(LruCache::new(NonZeroUsize::new(10_000).unwrap()))
});

use lazy_static::lazy_static; // 添加这一行
use tokio::sync::broadcast; // 添加这一行

lazy_static! {
    pub static ref WS_SENDER: broadcast::Sender<String> = {
        let (tx, _rx) = broadcast::channel(100);
        tx
    };
}

pub mod ws_server;

pub struct ShredStreamGrpc {
    shredstream_client: Arc<ShredstreamProxyClient<Channel>>,
}

struct TransactionWithSlot {
    transaction: VersionedTransaction,
    slot: u64,
}

impl ShredStreamGrpc {
    pub async fn new(endpoint: String) -> AnyResult<Self> {
        let shredstream_client = ShredstreamProxyClient::connect(endpoint.clone()).await?;
        Ok(Self { 
            shredstream_client: Arc::new(shredstream_client)
        })
    }

    pub async fn shredstream_subscribe<F>(&self, callback: F, bot_wallet: Option<Pubkey>) -> AnyResult<()> 
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
                                        transaction: transaction.clone(),
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

        while let Some(transaction_with_slot) = rx.next().await {
            if let Err(e) = Self::process_pumpfun_transaction(transaction_with_slot, &*callback, bot_wallet).await {
                error!("Error processing transaction: {:?}", e);
            }
        }
    
        Ok(())
    }

    async fn process_pumpfun_transaction<F>(transaction_with_slot: TransactionWithSlot, callback: &F, bot_wallet: Option<Pubkey>) -> AnyResult<()> 
    where
        F: Fn(PumpfunEvent) + Send + Sync,
    {
        let slot = transaction_with_slot.slot;
        let versioned_tx = transaction_with_slot.transaction;

        // 1. 提取 signature
        let signature = versioned_tx.signatures.get(0).map(|s| s.to_string()).unwrap_or_default();

        // 2. 去重：如果已处理过该tx，直接返回
        {
            let mut cache = PROCESSED_TXS.lock().unwrap();
            if cache.put(signature.clone(), ()).is_some() {
                // 已经处理过，直接跳过
                return Ok(());
            }
        } // 释放锁
        
        let mut token_info: Option<CreateTokenInfo> = None;
        let mut dev_trade_info: Option<TradeInfo> = None;
        let mut bonk_token_info: Option<BonkCreateTokenInfo> = None;
        let mut bonk_trade_info: Option<TradeRequest> = None;
        
        let instructions = LogFilter::parse_compiled_instruction(&versioned_tx, bot_wallet).unwrap();
        
        for instruction in instructions {
            match instruction {
                DexInstruction::CreateToken(mut token) => {
                    
                    let (limit, price, fee_merchant, fee) = LogFilter::parse_tip_info(&versioned_tx);
                    token.slot = slot;
                    token.unit_limit = limit.unwrap_or(0);
                    token.unit_price = price.unwrap_or(0);
                    token.fee_merchant = fee_merchant.unwrap_or_default();
                    token.fee = fee.unwrap_or(0);
                    token_info = Some(token);
                } 
                DexInstruction::BonkCreateToken(mut token) => {
                    
                    let (limit, price, fee_merchant, fee) = LogFilter::parse_tip_info(&versioned_tx);
                    token.unit_limit = limit.unwrap_or(0);
                    token.unit_price = price.unwrap_or(0);
                    token.fee_merchant = fee_merchant.unwrap_or_default();
                    token.fee = fee.unwrap_or(0);
                    bonk_token_info = Some(token);
                }
                DexInstruction::BonkTrade(mut trade_request) => {
                    bonk_trade_info = Some(trade_request);
                }
                DexInstruction::UserTrade(mut trade_info) => {
                    trade_info.slot = slot;
                    if token_info.is_some() {
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
        
        // 用if-else替代match
        match (token_info, dev_trade_info, bonk_token_info, bonk_trade_info) {
            (Some(token), Some(trade), None,None) => {
                let combined_event = PumpfunEvent::NewToken2 { token, trade };
                callback(combined_event);
            }
            (Some(token), None, None,None) => {
                callback(PumpfunEvent::NewToken(token));
            }
            (None, None, Some(bonk_token), Some(bonk_trade)) => {
                callback(PumpfunEvent::NewBonkToken{
                    token: bonk_token,
                    trade: bonk_trade,
                });
            }
            _ => {}
        }
        
        Ok(())
    }


    pub async fn process_pumpfun_transaction_shreder<F>(tx_info: &SubscribeTransactionsResponse, callback: &F) -> AnyResult<()>
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
        let tx_signature = bs58::encode(&tx_info.transaction.as_ref().unwrap().transaction.as_ref().unwrap().signatures[0]).into_string();

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
                    
                    let (limit, price, fee_merchant, fee) = LogFilter::parse_tip_info_shreder(&tx_info);
                    token.slot = slot;
                    token.unit_limit = limit.unwrap_or(0);
                    token.unit_price = price.unwrap_or(0);
                    token.fee_merchant = fee_merchant.unwrap_or_default();
                    token.fee = fee.unwrap_or(0);
                    token_info = Some(token);
                } 
                DexInstruction::BonkCreateToken(mut token) => {
                    
                    let (limit, price, fee_merchant, fee) = LogFilter::parse_tip_info_shreder(&tx_info);
                    token.unit_limit = limit.unwrap_or(0);
                    token.unit_price = price.unwrap_or(0);
                    token.fee_merchant = fee_merchant.unwrap_or_default();
                    token.fee = fee.unwrap_or(0);
                    bonk_token_info = Some(token);
                }
                DexInstruction::BonkTrade(mut trade_request) => {
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
            (Some(token), Some(trade), None,None) => {
                let combined_event = PumpfunEvent::NewToken2 { token, trade };
                callback(combined_event);
            }
            (Some(token), None, None,None) => {
                callback(PumpfunEvent::NewToken(token));
            }
            (None, None, Some(bonk_token), Some(bonk_trade)) => {
                callback(PumpfunEvent::NewBonkToken{
                    token: bonk_token,
                    trade: bonk_trade,
                });
            }
            _ => {}
        }
        
        Ok(())
    }
}
