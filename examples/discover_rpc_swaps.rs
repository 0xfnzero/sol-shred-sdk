mod common;
#[path = "common/rpc.rs"]
mod rpc;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let address = args.first().ok_or_else(|| {
        anyhow::anyhow!(
            "usage: discover_rpc_swaps <program-or-pool-address> <output-directory> [limit=20]"
        )
    })?;
    let output = args
        .get(1)
        .ok_or_else(|| anyhow::anyhow!("output directory required"))?;
    let limit = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(20);
    let client = rpc::client();
    let signatures = rpc::signatures(&client, address, limit)?;
    std::fs::create_dir_all(output)?;
    let mut found = 0;
    for signature in signatures {
        // Public RPC limits; discovery is an offline tooling path.
        std::thread::sleep(std::time::Duration::from_secs(2));
        let j = match rpc::fetch(&client, &signature) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("{signature}: {e}");
                continue;
            }
        };
        let (outer, route, checked) = match common::validate(&j) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{signature}: validation failed: {e}");
                continue;
            }
        };
        anyhow::ensure!(
            route.signature.to_string() == signature,
            "RPC signature mismatch"
        );
        // A referenced address does not prove the program executed.
        if !route
            .legs
            .iter()
            .any(|leg| leg.program.to_string() == *address || leg.pool.to_string() == *address)
        {
            continue;
        }
        let path = std::path::Path::new(output).join(format!("{signature}.json"));
        anyhow::ensure!(!path.exists(), "refusing to overwrite {}", path.display());
        std::fs::write(path, serde_json::to_vec_pretty(&j)?)?;
        println!(
            "{signature}: version={} slot={} outer={outer} swaps={} balance_checks={checked}",
            j["version"],
            j["slot"],
            route.legs.len()
        );
        found += 1;
    }
    anyhow::ensure!(found>0,"no validated swaps for this address; try a pool address, a larger limit, or an archival RPC");
    println!("saved {found} validated transactions");
    Ok(())
}
