use anyhow::{Context, Result};
use serde_json::{json, Value};
use solana_rpc_client::rpc_client::RpcClient;
use solana_rpc_client_api::request::RpcRequest;

pub fn client() -> RpcClient {
    RpcClient::new_with_timeout(
        std::env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into()),
        std::time::Duration::from_secs(20),
    )
}

pub fn fetch(client: &RpcClient, signature: &str) -> Result<Value> {
    let _: sol_shred_sdk::signature::Signature = signature.parse()?;
    let result: Value = client.send(
        RpcRequest::GetTransaction,
        json!([signature, {"encoding":"base64", "commitment":"finalized", "maxSupportedTransactionVersion":1}]),
    ).map_err(|_| anyhow::anyhow!("RPC getTransaction failed (endpoint details omitted)"))?;
    anyhow::ensure!(
        !result.is_null(),
        "transaction not found or not retained by RPC"
    );
    Ok(result)
}

#[allow(dead_code)] // Only discovery uses signature listing.
pub fn signatures(client: &RpcClient, address: &str, limit: usize) -> Result<Vec<String>> {
    let _: sol_shred_sdk::Pubkey = address.parse()?;
    anyhow::ensure!((1..=1000).contains(&limit), "limit must be 1..=1000");
    let result: Value = client
        .send(
            RpcRequest::GetSignaturesForAddress,
            json!([address, {"limit":limit, "commitment":"finalized"}]),
        )
        .map_err(|_| {
            anyhow::anyhow!("RPC getSignaturesForAddress failed (endpoint details omitted)")
        })?;
    result
        .as_array()
        .context("invalid signatures response")?
        .iter()
        .filter(|row| row.get("err").is_some_and(Value::is_null))
        .map(|row| {
            Ok(row["signature"]
                .as_str()
                .context("missing signature")?
                .to_owned())
        })
        .collect()
}
