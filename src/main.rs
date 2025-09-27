use shred_parsed::{PumpfunEvent, ShredStreamGrpc};
use serde_json::json; // 添加这个导入
use tonic::{Response, Streaming};
use futures::channel::mpsc::unbounded;  // 确保这个导入正确
use std::time::Instant;
use shred_parsed::grpc::{ws_server, WS_SENDER}; // 添加这个导入
use chrono::{DateTime, Utc}; // 添加chrono导入
use futures::SinkExt;  // 添加这个
use std::sync::Arc;  // 添加这个
use std::sync::atomic::{AtomicUsize, Ordering};  // 添加这个
use shred_parsed::common::logs_filters::LogFilter;
use shred_parsed::common::logs_data::{
    DexInstruction, 
    CreateTokenInfo, 
    TradeInfo, 
    BonkCreateTokenInfo, 
    TradeRequest
};

use shred_parsed::shredstream::{
    shreder_service_client::ShrederServiceClient, SubscribeRequestFilterTransactions,
    SubscribeTransactionsRequest, SubscribeTransactionsResponse,
};



use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use once_cell::sync::Lazy;
mod connector;
use clap::Parser;
use env_logger::Env;
pub mod decoder;

use connector::config::ClientConfig;

// 第17行，改为：

// 公共变量记录结果
// 在 main 函数外部定义静态变量
static ENTRIES_COUNT: AtomicUsize = AtomicUsize::new(0);
static TRANSACTIONS_COUNT: AtomicUsize = AtomicUsize::new(0);
// 只保留最近10000个tx的签名
static PROCESSED_TXS: Lazy<Mutex<LruCache<String, ()>>> = Lazy::new(|| {
    Mutex::new(LruCache::new(NonZeroUsize::new(10_000).unwrap()))
});
use lazy_static::lazy_static; // 添加这一行
use tokio::sync::broadcast; // 添加这一行
use std::fs::OpenOptions;
use std::io::Write;
use tokio::sync::watch;


fn write_mint_to_file(mint: &str) -> std::io::Result<()> {
    let timestamp: DateTime<Utc> = Utc::now();
    let line = format!("{}: {}\n", timestamp.format("%H:%M:%S%.3f"), mint);
    
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("mint_shred.txt")?;
    
    file.write_all(line.as_bytes())?;
    Ok(())
}


#[tokio::main]

async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let grpc = ShredStreamGrpc::new(
    //     "http://127.0.0.1:9999".to_string(), 
    // ).await.unwrap();
    // let grpc2 = ShredStreamGrpc::new("http://127.0.0.1:1999".to_string()).await?;
    tokio::spawn(async {
        ws_server::run_ws_server("127.0.0.1:8676").await;
    });

    let callback = |event: PumpfunEvent| {
        match event {
            // PumpfunEvent::NewDevTrade(trade_info) => {
            //     println!("Received new dev trade event: {:?}", trade_info);
            // },
            PumpfunEvent::NewToken(token_info) => {
                println!("Received new token 2fen event: {:?}", token_info);
                let tx_data = json!({
                    "mint": token_info.mint.to_string(),
                    "name": token_info.name,
                    "symbol": token_info.symbol,
                    "uri": token_info.uri,
                    "bonding_curve": token_info.bonding_curve.to_string(),
                    "user": token_info.user.to_string(),
                    "creator": token_info.creator.to_string(),
                    "sol_amount": 0,
                    "amount": 0,
                    "unit_limit": token_info.unit_limit,
                    "unit_price": token_info.unit_price,
                    "fee_merchant": token_info.fee_merchant,
                    "fee": token_info.fee,
                });
                write_mint_to_file(&token_info.mint.to_string()).unwrap();
 
                let _ = WS_SENDER.send(tx_data.to_string());
                println!("tx_data: {:?}", tx_data);
                if token_info.user.to_string() != token_info.creator.to_string() {
                   println!("mint: {} creator: {} user: {} 不同", token_info.mint.to_string(), token_info.creator.to_string(), token_info.user.to_string());
                }
            },
            PumpfunEvent::NewBonkToken{token, trade} => {
                // println!("Received new bonk token event: {:?}", token);
                // println!("Received new bonk trade event: {:?}", trade);

                let tx_data = json!({
                    "payer": token.payer,
                    "creator": token.creator,  // 需要从token信息获取
                    "base_mint": token.base_mint,
                    "pool_state": token.pool_state,  // 需要从token信息获取
                    "platform_config": token.platform_config,  // 需要从token信息获取
                    "virtual_base": token.virtual_base,
                    "virtual_quote":token.virtual_quote,
                    "base_vault": token.base_vault,  // 需要从token信息获取
                    "quote_vault": token.quote_vault,  // 需要从token信息获取
                    "symbol": token.symbol,  // 需要从token信息获取
                    "name": token.name,  // 需要从token信息获取
                    "uri": token.uri,  // 需要从token信息获取
                    "amount": trade.amount,
                    "trade_type": trade.trade_type as i32,
                    "unit_limit": token.unit_limit,  // 需要从token信息获取
                    "unit_price": token.unit_price,  // 需要从token信息获取
                    "fee_merchant": token.fee_merchant,
                    "fee": token.fee,
                    "type": 2,
                });
 
                let _ = WS_SENDER.send(tx_data.to_string());
                let timestamp: DateTime<Utc> = Utc::now();
                println!("[{}] Received new bonk token tx_data: {:?}", timestamp.format("%Y-%m-%d %H:%M:%S%.3f"), tx_data);
                if token.payer != token.creator {
                    println!("mint: {} creator: {} payer: {} 不同", token.base_mint, token.creator, token.payer);
                }
            },
            PumpfunEvent::NewUserTrade(trade_info) => {
                // println!("Received new trade event: {:?}", trade_info);
            },
            PumpfunEvent::NewToken2 { token, trade } => {
                // println!("Received new token2 event: {:?}", token);
                // println!("Received new trade event: {:?}", trade);
                write_mint_to_file(&token.mint.to_string()).unwrap();
                let tx_data = json!({
                    "mint": token.mint.to_string(),
                    "name": token.name,
                    "symbol": token.symbol,
                    "uri": token.uri,
                    "bonding_curve": token.bonding_curve.to_string(),
                    "user": token.user.to_string(),
                    "creator": token.creator.to_string(),
                    "sol_amount": trade.sol_amount,
                    "amount": trade.token_amount,
                    "unit_limit": token.unit_limit,
                    "unit_price": token.unit_price,
                    "fee_merchant": token.fee_merchant,
                    "fee": token.fee,
                    "type": 0,
                });
 
                let _ = WS_SENDER.send(tx_data.to_string());
                let timestamp: DateTime<Utc> = Utc::now();
                println!("[{}] Received new token2 2fen tx_data: {:?}", timestamp.format("%Y-%m-%d %H:%M:%S%.3f"), tx_data);
                if token.user.to_string() != token.creator.to_string() {
                    println!("mint: {} creator: {} user: {} 不同", token.mint.to_string(), token.creator.to_string(), token.user.to_string());
                }
            },
            PumpfunEvent::NewBotTrade(trade_info) => {
                println!("Received new bot trade event: {:?}", trade_info);
            },
            PumpfunEvent::Error(err) => {
                println!("Received error: {}", err);
            }
        }
    };

    // env_logger::Builder::from_env(Env::default().default_filter_or("info"))
    //     .format_timestamp_secs()
    //     .init();

    // let config = ClientConfig::parse();

    // log::info!("Starting JetStream Example Client");

    // let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // // if config.parsed_enabled {
    // //     connector::parsed::jetstream_parsed_connector(config, shutdown_rx).await?;
    // // } else {
    // connector::connector::jetstream_connector(callback,config).await?;
    // // }

    // tokio::spawn(async move {
    //     match tokio::signal::ctrl_c().await {
    //         Ok(()) => {
    //             log::info!("Received Ctrl+C signal, initiating shutdown...");
    //             let _ = shutdown_tx.send(true);
    //             Ok(())
    //         }
    //         Err(err) => {
    //             eprintln!("Error setting up Ctrl+C handler: {}", err);
    //             Err(())
    //         }
    //     }
    // });




    
    // let shared_stats_clone = Arc::clone(&*SHARED_TX_STATS);
    
    let entrypoint = "http://fra1.shreder.xyz:9991/";
    println!("正在连接到: {}", entrypoint);

    let mut client = ShrederServiceClient::connect(entrypoint).await.unwrap();
    println!("成功连接到 shreder 服务");

    let request = SubscribeTransactionsRequest {
        transactions: maplit::hashmap! {
            "pumpfun".to_owned() => SubscribeRequestFilterTransactions {
                account_exclude: vec![],
                account_include: vec![],
                account_required: vec!["6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P".to_owned()]
            }
        },
    };
    println!("发送订阅请求: {:?}", request);

    let (mut subscribe_tx, subscribe_rx) = unbounded();
    let response: Response<Streaming<SubscribeTransactionsResponse>> =
        client.subscribe_transactions(subscribe_rx).await.unwrap();
    println!("成功建立订阅流");

    let mut stream = response.into_inner();
    println!("开始监听交易流...");

    let _ = subscribe_tx.send(request).await;
    println!("订阅请求已发送，等待数据...");

    
    
    let mut message_count = 0;
    while let Some(message) = stream.message().await.unwrap() {
        message_count += 1;
        // println!("收到消息 #{}", message_count);
        
        let mut token_info: Option<CreateTokenInfo> = None;
        let mut dev_trade_info: Option<TradeInfo> = None;
        let mut bonk_token_info: Option<BonkCreateTokenInfo> = None;
        let mut bonk_trade_info: Option<TradeRequest> = None;
        let slot = 0; // 或者从其他地方获取
        
        if let Some(proto_tx) = &message.transaction {
            // println!("消息包含交易数据");
            let boxed_callback = Box::new(callback);
            if let Some(tx) = &proto_tx.transaction {
                ShredStreamGrpc::process_pumpfun_transaction_shreder(&message, &*boxed_callback).await.unwrap();

                // let tx_signature = bs58::encode(&tx.signatures[0]).into_string();
                // // println!("处理交易签名: {}", tx_signature);
                // // 2. 去重：如果已处理过该tx，直接返回
                // {
                //     let mut cache = PROCESSED_TXS.lock().unwrap();
                //     if cache.put(tx_signature.clone(), ()).is_some() {
                //         // 已经处理过，直接跳过
                //         continue;
                //     }
                // } 
                // let instructions = LogFilter::parse_compiled_instruction_shreder(&message).unwrap();
                // for instruction in instructions {
                //     match instruction {
                //         DexInstruction::CreateToken(mut token) => {
                            
                //             let (limit, price, fee_merchant, fee) = LogFilter::parse_tip_info(&versioned_tx);
                //             token.slot = slot;
                //             token.unit_limit = limit.unwrap_or(0);
                //             token.unit_price = price.unwrap_or(0);
                //             token.fee_merchant = fee_merchant.unwrap_or_default();
                //             token.fee = fee.unwrap_or(0);
                //             token_info = Some(token);
                //         } 
                //         DexInstruction::BonkCreateToken(mut token) => {
                            
                //             // let (limit, price, fee_merchant, fee) = LogFilter::parse_tip_info(&versioned_tx);
                //             // token.unit_limit = limit.unwrap_or(0);
                //             // token.unit_price = price.unwrap_or(0);
                //             // token.fee_merchant = fee_merchant.unwrap_or_default();
                //             // token.fee = fee.unwrap_or(0);
                //             bonk_token_info = Some(token);
                //         }
                //         DexInstruction::BonkTrade(mut trade_request) => {
                //             bonk_trade_info = Some(trade_request);
                //         }
                //         DexInstruction::UserTrade(mut trade_info) => {
                //             // trade_info.slot = slot;
                //             if token_info.is_some() {
                //                 dev_trade_info = Some(trade_info);
                //             } 
                //         }
                //         DexInstruction::BotTrade(mut trade_info) => {
                //             // trade_info.slot = slot;
                //             callback(PumpfunEvent::NewBotTrade(trade_info));
                //         }
                //         _ => {}
                //     }
                // }
                
                // // 用if-else替代match
                // match (&token_info, &dev_trade_info, &bonk_token_info, &bonk_trade_info) {
                //     (Some(token), Some(trade), None, None) => {
                //         let combined_event = PumpfunEvent::NewToken2 { 
                //             token: token.clone(), 
                //             trade: trade.clone() 
                //         };
                //         callback(combined_event);
                //     }
                //     (Some(token), None, None, None) => {
                //         callback(PumpfunEvent::NewToken(token.clone()));
                //     }
                //     (None, None, Some(bonk_token), Some(bonk_trade)) => {
                //         callback(PumpfunEvent::NewBonkToken{
                //             token: bonk_token.clone(),
                //             trade: bonk_trade.clone(),
                //         });
                //     }
                //     _ => {}
                // }
                
                // //打印交易的基本信息
                // println!("=== 交易信息 ===");
                // println!("交易签名: {}", tx_signature);
                // println!("签名数量: {}", tx.signatures.len());
                
                // // 使用 if let 来处理 Option<Message>
                // if let Some(message) = &tx.message {
                //     // 打印消息头信息
                //     if let Some(header) = &message.header {
                //         println!("=== 消息头 ===");
                //         println!("必需签名数量: {}", header.num_required_signatures);
                //         println!("只读签名账户数量: {}", header.num_readonly_signed_accounts);
                //         println!("只读非签名账户数量: {}", header.num_readonly_unsigned_accounts);
                //     }
                    
                //     // 打印账户公钥
                //     println!("=== 账户公钥 ===");
                //     println!("账户数量: {}", message.account_keys.len());
                //     for (i, key) in message.account_keys.iter().enumerate() {
                //         let pubkey = bs58::encode(key).into_string();
                //         println!("账户 {}: {}", i, pubkey);
                //     }
                    
                //     // 打印指令信息
                //     println!("=== 指令信息 ===");
                //     println!("指令数量: {}", message.instructions.len());
                //     for (i, instruction) in message.instructions.iter().enumerate() {
                //         println!("指令 {}:", i);
                //         println!("  程序ID索引: {}", instruction.program_id_index);
                //         println!("  账户索引数量: {}", instruction.accounts.len());
                //         println!("  数据长度: {} 字节", instruction.data.len());
                        
                //         // 打印指令数据（前16字节的十六进制表示）
                //         if !instruction.data.is_empty() {
                //             let hex_data: String = instruction.data.iter()
                //                 .take(16)
                //                 .map(|b| format!("{:02x}", b))
                //                 .collect::<Vec<String>>()
                //                 .join(" ");
                //             println!("  数据 (前16字节): {}", hex_data);
                //         }
                //     }
                    
                //     // 打印其他信息
                //     println!("=== 其他信息 ===");
                //     println!("版本化: {}", message.versioned);
                //     println!("地址表查找数量: {}", message.address_table_lookups.len());
                    
                //     if !message.recent_blockhash.is_empty() {
                //         let blockhash = bs58::encode(&message.recent_blockhash).into_string();
                //         println!("最近区块哈希: {}", blockhash);
                //     } else {
                //         println!("最近区块哈希: 空");
                //     }
                
                
                // println!("=== 交易信息结束 ===\n");
            } else {
                println!("proto_tx.transaction 为空");
            }
        } else {
            println!("消息中没有 transaction 字段");
        }
    }

    // println!("流结束，总共收到 {} 条消息", message_count);

    
    // grpc.shredstream_subscribe(callback, None).await.unwrap();
    Ok(())
  

    // 主线程阻塞在第二个订阅
    // grpc2.shredstream_subscribe(callback, None).await?;

}