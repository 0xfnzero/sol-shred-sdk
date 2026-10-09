#[path = "common/instructions.rs"]
mod instructions;
#[path = "common/rpc.rs"]
mod rpc;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let input = args.first().ok_or_else(|| {
        anyhow::anyhow!("usage: rpc_parse_liquidity <signature|fixture.json> [save.json]")
    })?;
    let j: serde_json::Value = if input.ends_with(".json") {
        serde_json::from_slice(&std::fs::read(input)?)?
    } else {
        rpc::fetch(&rpc::client(), input)?
    };
    let events = instructions::validate(&j)?;
    anyhow::ensure!(
        !events.is_empty(),
        "no supported LP or creator-fee collection instruction found"
    );
    let raw = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        j["transaction"][0].as_str().unwrap(),
    )?;
    let tx: sol_shred_sdk::transaction::VersionedTransaction = wincode::deserialize(&raw)?;
    if !input.ends_with(".json") {
        anyhow::ensure!(tx.signatures[0].to_string() == *input, "signature mismatch");
    }
    if let Some(path) = args.get(1) {
        std::fs::write(path, serde_json::to_vec_pretty(&j)?)?;
    }
    println!(
        "signature={} slot={} succeeded={} checked_instructions={}",
        tx.signatures[0],
        j["slot"],
        j["meta"]["err"].is_null(),
        events.len()
    );
    println!("{}", serde_json::to_string_pretty(&events)?);
    Ok(())
}
