use anyhow::{ensure, Context, Result};
use base64::Engine;
use serde_json::Value;
use sol_shred_sdk::transaction::VersionedTransaction;

/// Cold-path evidence validation; signature verification stays outside SDK parsing.
pub fn signed_transaction(j: &Value) -> Result<VersionedTransaction> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(
        j["transaction"][0]
            .as_str()
            .context("base64 transaction missing")?,
    )?;
    let tx: VersionedTransaction = wincode::deserialize_exact(&bytes)?;
    ensure!(!tx.signatures.is_empty(), "missing signature");
    tx.sanitize().context("invalid transaction shape")?;
    tx.verify_and_hash_message()
        .context("invalid transaction signature")?;
    Ok(tx)
}
