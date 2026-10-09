#[path = "common/rpc.rs"]
mod rpc;
#[path = "common/pumpswap_verify.rs"]
mod verify;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let input = args.first().ok_or_else(|| {
        anyhow::anyhow!("usage: rpc_verify_pumpswap <signature|fixture.json> [save.json]")
    })?;
    let j: serde_json::Value = if input.ends_with(".json") {
        serde_json::from_slice(&std::fs::read(input)?)?
    } else {
        rpc::fetch(&rpc::client(), input)?
    };
    let report = verify::verify(&j)?;
    if !input.ends_with(".json") {
        anyhow::ensure!(report.signature == *input, "RPC signature mismatch");
    }
    if let Some(path) = args.get(1) {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&j)?)?;
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
