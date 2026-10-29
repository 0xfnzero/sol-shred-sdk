//! Subscribe to Jito-style ShredStream gRPC entries with optional x-token auth.
//! Set SHREDSTREAM_ENDPOINT and, if required, SHREDSTREAM_X_TOKEN.
use sol_shred_sdk::{ShredStreamClient, ShredStreamConfig};

fn main() -> anyhow::Result<()> {
    let endpoint = std::env::var("SHREDSTREAM_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:10000".to_owned());
    let config = ShredStreamConfig {
        auth_token: std::env::var("SHREDSTREAM_X_TOKEN").ok(),
        ..ShredStreamConfig::jito_grpc(endpoint)
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let client = ShredStreamClient::new_with_config(config).await?;
            // Display only: production callbacks should avoid blocking output.
            client
                .subscribe_callback(|event| println!("{event:?}"))
                .await?;
            std::future::pending::<()>().await;
            Ok(())
        })
}
