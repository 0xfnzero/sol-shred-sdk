use jetstream_protos::jetstream::jetstream_client::JetstreamClient;
use jetstream_protos::jetstream::SubscribeParsedRequest;
use solana_sdk::bs58;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};
use tokio_stream::StreamExt;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

use super::config::ClientConfig;

pub async fn jetstream_parsed_connector(
    config: ClientConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let url_str = config.jetstream_grpc_url.clone();

    let channel = Channel::builder(url_str.parse()?).connect().await?;

    let x_token: MetadataValue<_> = config.x_token.clone().unwrap_or_default().parse()?;

    let mut client = JetstreamClient::new(channel.clone());

    let has_token = !config.x_token.clone().unwrap_or_default().is_empty();

    log::info!("Jetstream parsed connector connected successfully");

    let request = SubscribeParsedRequest {
        ping: Some(jetstream_protos::jetstream::SubscribeRequestPing { id: 1 }),
    };

    let outbound = tokio_stream::iter(vec![request]);

    let response_result = if has_token {
        let mut request = tonic::Request::new(outbound);
        request.metadata_mut().insert("x-token", x_token.clone());
        timeout(Duration::from_secs(5), client.subscribe_parsed(request)).await
    } else {
        timeout(Duration::from_secs(5), client.subscribe_parsed(outbound)).await
    };

    let response = match response_result {
        Ok(Ok(response)) => response,
        Ok(Err(e)) => return Err(anyhow::anyhow!("Failed to subscribe: {}", e)),
        Err(_) => return Err(anyhow::anyhow!("Subscribe timeout")),
    };

    let mut inbound = response.into_inner();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10000);

    let mut shutdown_rx_logger = shutdown_rx.clone();
    let logging_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(log_message) = rx.recv() => {
                    log::info!("{}", log_message);
                }
                Ok(()) = shutdown_rx_logger.changed() => {
                    if *shutdown_rx_logger.borrow() {
                        log::info!("Jetstream parsed logging task shutting down");
                        break;
                    }
                }
                else => break
            }
        }
    });

    let mut spawned_tasks: Vec<JoinHandle<()>> = Vec::new();

    loop {
        tokio::select! {
            Some(response) = inbound.next() => {
                let tx_clone = tx.clone();

                // Process each message in its own task
                let task = tokio::spawn(async move {
                    if let Ok(parsed_tx) = response {
                        if !parsed_tx.signature.is_empty() {
                            let signature = bs58::encode(&parsed_tx.signature).into_string();

                            // Extract and format instruction types for display only
                            let instruction_summary = if !parsed_tx.instructions.is_empty() {
                                let instruction_types: Vec<String> = parsed_tx.instructions.iter()
                                    .filter_map(|instruction| {
                                        match &instruction.instruction_oneof {
                                            Some(jetstream_protos::jetstream::instruction::InstructionOneof::Initialize(_)) => {
                                                Some("initialize".to_string())
                                            }
                                            Some(jetstream_protos::jetstream::instruction::InstructionOneof::SetParams(_)) => {
                                                Some("set_params".to_string())
                                            }
                                            Some(jetstream_protos::jetstream::instruction::InstructionOneof::Create(_)) => {
                                                Some("create".to_string())
                                            }
                                            Some(jetstream_protos::jetstream::instruction::InstructionOneof::Buy(_)) => {
                                                Some("buy".to_string())
                                            }
                                            Some(jetstream_protos::jetstream::instruction::InstructionOneof::Sell(_)) => {
                                                Some("sell".to_string())
                                            }
                                            Some(jetstream_protos::jetstream::instruction::InstructionOneof::Withdraw(_)) => {
                                                Some("withdraw".to_string())
                                            }
                                            None => None,
                                        }
                                    })
                                    .collect();

                                format!(", Instructions: {}", instruction_types.join(", "))
                            } else {
                                "".to_string()
                            };
                            if instruction_summary.contains("initialize") {
                                let log_message = format!(
                                    "Jetstream Parsed - Transaction received - Signature: {}{}",
                                    signature,
                                    instruction_summary
                                );
    
                                let _ = tx_clone.try_send(log_message);
                                
                            }
                            
                        }
                    }
                });

                spawned_tasks.push(task);
                spawned_tasks.retain(|task| !task.is_finished());
            }
            Ok(()) = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    log::info!("Jetstream parsed connector shutting down due to signal");
                    for task in spawned_tasks {
                        task.abort();
                    }
                    break;
                }
            }
            else => {
                log::info!("Jetstream parsed connector stream ended");
                break;
            }
        }
    }

    drop(inbound);
    drop(client);

    if !logging_task.is_finished() {
        match timeout(Duration::from_secs(2), logging_task).await {
            Ok(_) => log::info!("Logging task completed"),
            Err(_) => {
                log::warn!("Logging task did not complete within timeout");
            }
        }
    }

    log::info!("Jetstream parsed connector shutdown complete");
    Ok(())
}
