use solana_pubsub_client::nonblocking::pubsub_client::{PubsubClient, PubsubClientResult};
use solana_rpc_client_api::config::{RpcTransactionLogsConfig, RpcTransactionLogsFilter};

use crate::common::{logs_data::DexInstruction, logs_filters::LogFilter};
use futures::StreamExt;
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use super::logs_events::PumpfunEvent;

const SUBSCRIPTION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Subscription handle containing task and unsubscribe logic
pub struct SubscriptionHandle {
    pub task: JoinHandle<()>,
    pub unsub_fn: Box<dyn Fn() + Send>,
}

impl SubscriptionHandle {
    pub async fn shutdown(self) {
        let Self { mut task, unsub_fn } = self;
        unsub_fn();
        if timeout(SUBSCRIPTION_SHUTDOWN_TIMEOUT, &mut task)
            .await
            .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
}

pub async fn create_pubsub_client(ws_url: &str) -> PubsubClientResult<PubsubClient> {
    PubsubClient::new(ws_url).await
}

/// 启动订阅
pub async fn tokens_subscription<F>(
    ws_url: &str,
    commitment: CommitmentConfig,
    callback: F,
    bot_wallet: Option<Pubkey>,
) -> Result<SubscriptionHandle, Box<dyn std::error::Error>>
where
    F: Fn(PumpfunEvent) + Send + Sync + 'static,
{
    let program_address = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P".to_string();
    let logs_filter = RpcTransactionLogsFilter::Mentions(vec![program_address]);

    let logs_config = RpcTransactionLogsConfig {
        commitment: Some(commitment),
    };

    let sub_client = PubsubClient::new(ws_url).await?;

    let (unsub_tx, mut unsub_rx) = mpsc::channel(1);
    let (ready_tx, ready_rx) = oneshot::channel();

    let task = tokio::spawn(async move {
        let (mut stream, unsubscribe) =
            match sub_client.logs_subscribe(logs_filter, logs_config).await {
                Ok(subscription) => subscription,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
        if ready_tx.send(Ok(())).is_err() {
            let _ = timeout(SUBSCRIPTION_SHUTDOWN_TIMEOUT, unsubscribe()).await;
            return;
        }

        loop {
            tokio::select! {
                _ = unsub_rx.recv() => break,
                msg = stream.next() => {
                    let Some(msg) = msg else {
                        log::warn!("token log subscription stream ended");
                        break;
                    };

                    if let Some(_err) = msg.value.err {
                        continue;
                    }

                    let instructions = match LogFilter::parse_instruction(&msg.value.logs, bot_wallet) {
                        Ok(instructions) => instructions,
                        Err(error) => {
                            log::debug!("failed to parse token subscription logs: {error}");
                            continue;
                        }
                    };
                    for instruction in instructions {
                        match instruction {
                            DexInstruction::CreateToken(token_info) => {
                                callback(PumpfunEvent::NewToken(token_info));
                            }
                            DexInstruction::UserTrade(trade_info) => {
                                callback(PumpfunEvent::NewUserTrade(trade_info));
                            }
                            DexInstruction::BotTrade(trade_info) => {
                                callback(PumpfunEvent::NewBotTrade(trade_info));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if timeout(SUBSCRIPTION_SHUTDOWN_TIMEOUT, unsubscribe())
            .await
            .is_err()
        {
            log::warn!("token log unsubscribe timed out");
        }
    });

    match ready_rx.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(Box::new(error)),
        Err(error) => return Err(Box::new(error)),
    }

    Ok(SubscriptionHandle {
        task,
        unsub_fn: Box::new(move || {
            let _ = unsub_tx.try_send(());
        }),
    })
}

pub async fn stop_subscription(handle: SubscriptionHandle) {
    handle.shutdown().await;
}
