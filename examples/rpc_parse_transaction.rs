mod common;
use anyhow::{Context, Result};
#[path = "common/rpc.rs"]
mod rpc;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let input = args
        .first()
        .context("usage: rpc_parse_transaction <signature|fixture.json> [save.json]")?;
    let j: serde_json::Value = if input.ends_with(".json") {
        serde_json::from_slice(&std::fs::read(input)?)?
    } else {
        rpc::fetch(&rpc::client(), input)?
    };
    let (outer, route, checked) = common::validate(&j)?;
    if !input.ends_with(".json") {
        anyhow::ensure!(
            route.signature.to_string() == *input,
            "RPC returned a different signature"
        );
    }
    if let Some(path) = args.get(1) {
        std::fs::write(path, serde_json::to_vec_pretty(&j)?)?;
    }
    println!("signature={} slot={} succeeded={} outer_events={} rpc_swap_legs={} verified_balance_accounts={}",route.signature,j["slot"],route.succeeded,outer,route.legs.len(),checked);
    println!("{}", serde_json::to_string_pretty(&route)?);
    Ok(())
}
