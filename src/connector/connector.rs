use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use sol_shred_sdk::common::logs_filters::LogFilter;
use sol_shred_sdk::common::logs_data::DexInstruction;
use sol_shred_sdk::PumpfunEvent;
use sol_shred_sdk::grpc::ShredStreamGrpc;
use std::path::PathBuf;  // 添加这个导入
use sol_shred_sdk::shredstream::SubscribeTransactionsResponse;

use jetstream_protos::jetstream::{
    jetstream_client::JetstreamClient, subscribe_update::UpdateOneof, SubscribeRequest,
    SubscribeRequestFilterTransactions, SubscribeUpdateTransactionInfo,
};
use solana_sdk::{bs58, pubkey::Pubkey};
use tokio_stream::StreamExt;
use tonic::{transport::Channel, Request};

use crate::decoder::pumpfun::PumpProgramIx;

use super::config::{ClientConfig, FilterConfig};


pub async fn jetstream_connector<F>(callback: F, mut config: ClientConfig) -> anyhow::Result<()> 
where
    F: Fn(PumpfunEvent) + Send + Sync + 'static,
{
    log::info!(
        "Starting Jetstream connector with URL: {}",
        config.jetstream_grpc_url
    );
    let grpc_url = "http://ny.jetstream.orbitflare.com";
    config.filter_config_path = Some(PathBuf::from("filters.json"));
    let channel = Channel::from_shared(grpc_url)?.connect().await?;

    let token = config.x_token.clone().unwrap_or_default();
    let mut client = JetstreamClient::with_interceptor(channel, move |mut req: Request<()>| {
        if !token.is_empty() {
            req.metadata_mut()
                .insert("authorization", token.parse().unwrap());
        }
        Ok(req)
    });

    log::info!("Jetstream connector connected successfully");

    let filters = build_filters(&config)?;
    log::info!(
        "Using {} filter(s) for transaction filtering",
        filters.len()
    );

    for (name, filter) in &filters {
        log::info!("Filter '{}' configuration:", name);
        if !filter.account_include.is_empty() {
            log::info!("  Include accounts: {}", filter.account_include.join(", "));
        }
        if !filter.account_exclude.is_empty() {
            log::info!("  Exclude accounts: {}", filter.account_exclude.join(", "));
        }
        if !filter.account_required.is_empty() {
            log::info!(
                "  Required accounts: {}",
                filter.account_required.join(", ")
            );
        }
    }

    let request = SubscribeRequest {
        transactions: filters,
        accounts: HashMap::new(),
        ping: Some(jetstream_protos::jetstream::SubscribeRequestPing { id: 1 }),
    };

    let outbound = tokio_stream::iter(vec![request]);
    let response = client.subscribe(outbound).await?;
    let mut inbound = response.into_inner();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<SubscribeTransactionsResponse>(10000);

    tokio::spawn(async move {
        while let Some(tx_info) = rx.recv().await {
            ShredStreamGrpc::process_pumpfun_transaction_shreder(&tx_info, &callback).await.unwrap();
            
            // log::info!(
            //     "Jetstream - Transaction received - Signature: {}",
            //     bs58::encode(&tx_info.signature).into_string()
            // );
            // for instruction in tx_info.instructions {
            //     let accounts: Vec<Pubkey> = tx_info
            //         .account_keys
            //         .iter()
            //         .map(|i| {
            //             let mut array = [0; 32];
            //             let bytes = &i[..array.len()];
            //             array.copy_from_slice(bytes);
            //             Pubkey::new_from_array(array)
            //         })
            //         .collect();

            //     match PumpProgramIx::deserialize_pumpfun(accounts, &instruction.data) {
            //         Ok(ix) => {
            //             if let PumpProgramIx::Create(_) = ix {
            //                 println!(
            //                     "[{}] Signature: {} - Pump program ix: {:#?}",
            //                     chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            //                     bs58::encode(&tx_info.signature).into_string(),
            //                     ix
            //                 );
            //             }
            //         }
            //         Err(_) => {}
            //     }
            // }
        }
    });

    // while let Some(response) = inbound.next().await {
    //     let tx_clone = tx.clone();
    //     tokio::spawn(async move {
    //         if let Ok(msg) = response {
    //             if let Some(UpdateOneof::Transaction(tx_update)) = msg.update_oneof {
    //                 if let Some(tx_info) = tx_update.transaction {
    //                     let _ = tx_clone.try_send(tx_info);
    //                 }
    //             }
    //         }
    //     });
    // }

    log::info!("Jetstream connector stream ended");
    Ok(())
}

/// Build transaction filters from config
fn build_filters(
    config: &ClientConfig,
) -> anyhow::Result<HashMap<String, SubscribeRequestFilterTransactions>> {
    let mut filters = HashMap::new();

    // Try to load filters from file if specified
    if let Some(filter_path) = &config.filter_config_path {
        log::info!("Loading filters from file: {}", filter_path.display());
        let file = File::open(filter_path)?;
        let reader = BufReader::new(file);
        let filter_config: FilterConfig = serde_json::from_reader(reader)?;

        // Convert each filter to the protobuf format
        for (name, filter) in filter_config.filters {
            filters.insert(
                name,
                SubscribeRequestFilterTransactions {
                    account_include: filter.account_include,
                    account_exclude: filter.account_exclude,
                    account_required: filter.account_required,
                },
            );
        }
    }
    // If no file but command-line filters provided, use those
    else if config.include_accounts.is_some()
        || config.exclude_accounts.is_some()
        || config.required_accounts.is_some()
    {
        log::info!("Using command-line filter configuration");
        filters.insert(
            "cli-filter".to_string(),
            SubscribeRequestFilterTransactions {
                account_include: config.include_accounts.clone().unwrap_or_default(),
                account_exclude: config.exclude_accounts.clone().unwrap_or_default(),
                account_required: config.required_accounts.clone().unwrap_or_default(),
            },
        );
    }
    // Default case - empty filter (all transactions)
    else {
        log::info!("No filters specified, using default filter (all transactions)");
        filters.insert(
            "default".to_string(),
            SubscribeRequestFilterTransactions {
                account_include: vec![],
                account_exclude: vec![],
                account_required: vec![],
            },
        );
    }

    Ok(filters)
}
