#[path = "common/rpc.rs"]
mod rpc;
#[path = "common/pumpswap_verify.rs"]
mod verify;

fn main() -> anyhow::Result<()> {
    use std::io::Write;
    let args: Vec<_> = std::env::args().skip(1).collect();
    let output = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("usage: rpc_find_pumpswap <output-directory> [limit=20]"))?;
    let limit = args.get(1).map(|n| n.parse()).transpose()?.unwrap_or(20);
    let client = rpc::client();
    let signatures = rpc::signatures(
        &client,
        &sol_shred_sdk::instr::pump_amm::PROGRAM_ID_PUBKEY.to_string(),
        limit,
    )?;
    std::fs::create_dir_all(output)?;
    let mut saved = 0;
    for signature in signatures {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let result = rpc::fetch(&client, &signature)
            .and_then(|j| verify::verify(&j).map(|report| (j, report)));
        let (j, report) = match result {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{signature}: {e}");
                continue;
            }
        };
        anyhow::ensure!(report.signature == signature, "RPC signature mismatch");
        let path = std::path::Path::new(output).join(format!("{signature}.json"));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&j)?)?;
        println!("{}", serde_json::to_string(&report)?);
        saved += 1;
    }
    anyhow::ensure!(
        saved > 0,
        "no verified PumpSwap trades found; increase limit or use an archival RPC"
    );
    println!("saved {saved} verified RPC transactions");
    Ok(())
}
