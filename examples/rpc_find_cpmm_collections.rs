#[path = "common/instructions.rs"]
mod instructions;
#[path = "common/rpc.rs"]
mod rpc;
use anyhow::{ensure, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use sol_shred_sdk::Pubkey;
use solana_rpc_client_api::request::RpcRequest;

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let pool = args
        .first()
        .context("usage: rpc_find_cpmm_collections <pool-address> <output-directory> [limit=10]")?;
    let _: Pubkey = pool.parse()?;
    let output = args.get(1).context("output directory required")?;
    let limit = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(10);
    let client = rpc::client();
    let response: Value = client
        .send(
            RpcRequest::GetAccountInfo,
            json!([pool,{"encoding":"base64","commitment":"finalized"}]),
        )
        .map_err(|_| anyhow::anyhow!("RPC getAccountInfo failed (endpoint details omitted)"))?;
    let account = &response["value"];
    ensure!(
        account["owner"] == "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C",
        "not a CPMM-owned pool account"
    );
    let data = base64::engine::general_purpose::STANDARD.decode(
        account["data"][0]
            .as_str()
            .context("pool account missing")?,
    )?;
    ensure!(
        data.len() >= 637 && data[..8] == [247, 237, 227, 245, 215, 195, 222, 70],
        "invalid CPMM PoolState layout"
    );
    let config = Pubkey::new_from_array(data[8..40].try_into()?);
    let creator = Pubkey::new_from_array(data[40..72].try_into()?);
    let share = sol_shred_sdk::cpmm_creator_fee::derive_creator_fee_share(&creator, &config).0;
    println!("pool={pool} creator={creator} amm_config={config} creator_fee_share={share}");
    // The PDA may be absent as an account and still appear in collection calls.
    let signatures = rpc::signatures(&client, &share.to_string(), limit)?;
    std::fs::create_dir_all(output)?;
    let mut found = 0;
    for signature in signatures {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let j = rpc::fetch(&client, &signature)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(j["transaction"][0].as_str().context("transaction")?)?;
        let tx: sol_shred_sdk::transaction::VersionedTransaction = wincode::deserialize(&bytes)?;
        ensure!(
            tx.signatures
                .first()
                .is_some_and(|s| s.to_string() == signature),
            "signature mismatch"
        );
        let events = instructions::validate(&j)?;
        let matching: Vec<_> = events
            .iter()
            .filter(|e| {
                e["kind"] == "RaydiumCpmmCollectCreatorFee" && e["accounts"]["pool_state"] == *pool
            })
            .collect();
        if matching.is_empty() {
            continue;
        }
        let path = std::path::Path::new(output).join(format!("{signature}.json"));
        ensure!(!path.exists(), "refusing to overwrite {}", path.display());
        std::fs::write(path, serde_json::to_vec_pretty(&j)?)?;
        println!(
            "signature={signature} slot={} checked_collections={}",
            j["slot"],
            matching.len()
        );
        found += 1;
    }
    println!("saved {found} validated collection transactions for this pool");
    Ok(())
}
