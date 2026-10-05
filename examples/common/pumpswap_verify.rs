//! RPC-only validation tooling: wire checks are independent of SDK decoder helpers.
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use serde_json::Value;
use sol_shred_sdk::{
    accounts::pumpswap::effective_quote_reserves,
    instr::{pump_amm, pump_amm_inner},
    logs::pump_amm as logs,
    transaction::VersionedTransaction,
    DexEvent, EventMetadata, Pubkey,
};

#[derive(Debug, Serialize)]
pub struct Report {
    pub signature: String,
    pub slot: u64,
    pub version: Value,
    pub checked_instructions: usize,
    pub checked_log_cpi_pairs: usize,
    pub trades: Vec<Trade>,
}

#[derive(Debug, Serialize)]
pub struct Trade {
    pub side: &'static str,
    pub pool: String,
    pub base_reserve: u64,
    pub raw_quote_reserve: u64,
    pub virtual_quote_reserve: String,
    pub effective_quote_reserve: u64,
}

fn u64_at(data: &[u8], offset: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        data.get(offset..offset + 8)
            .context("wire u64 truncated")?
            .try_into()?,
    ))
}

fn normalize(event: &mut DexEvent) {
    // CPI Sell has a derived is_pump_pool flag; it is not part of the wire event.
    if let DexEvent::PumpSwapSell(e) = event {
        e.is_pump_pool = false;
    }
}

fn supported_event(bytes: &[u8]) -> bool {
    let Some(prefix) = bytes.get(..8) else {
        return false;
    };
    let disc = u64::from_le_bytes(prefix.try_into().expect("eight-byte discriminator"));
    matches!(
        disc,
        logs::discriminators::BUY
            | logs::discriminators::SELL
            | logs::discriminators::CREATE_POOL
            | logs::discriminators::ADD_LIQUIDITY
            | logs::discriminators::REMOVE_LIQUIDITY
    )
}

fn check_trade(event: &DexEvent, bytes: &[u8]) -> Result<Trade> {
    let (side, pool, user, timestamp, base, quote, virtual_reserve, amount, limit, actual, tail) =
        match event {
            DexEvent::PumpSwapBuy(e) => {
                let name_len =
                    u32::from_le_bytes(bytes.get(393..397).context("name length")?.try_into()?)
                        as usize;
                (
                    "buy",
                    e.pool,
                    e.user,
                    e.timestamp,
                    e.pool_base_token_reserves,
                    e.pool_quote_token_reserves,
                    e.virtual_quote_reserves,
                    e.base_amount_out,
                    e.max_quote_amount_in,
                    e.quote_amount_in,
                    397usize.checked_add(name_len).context("name overflow")?,
                )
            }
            DexEvent::PumpSwapSell(e) => (
                "sell",
                e.pool,
                e.user,
                e.timestamp,
                e.pool_base_token_reserves,
                e.pool_quote_token_reserves,
                e.virtual_quote_reserves,
                e.base_amount_in,
                e.min_quote_amount_out,
                e.quote_amount_out,
                352,
            ),
            _ => anyhow::bail!("not a trade"),
        };
    ensure!(
        timestamp == i64::from_le_bytes(bytes.get(..8).context("timestamp truncated")?.try_into()?),
        "timestamp mismatch"
    );
    ensure!(
        pool == Pubkey::new_from_array(bytes.get(112..144).context("pool truncated")?.try_into()?)
            && user
                == Pubkey::new_from_array(
                    bytes.get(144..176).context("user truncated")?.try_into()?
                ),
        "event account mismatch"
    );
    ensure!(
        amount == u64_at(bytes, 8)? && limit == u64_at(bytes, 16)? && actual == u64_at(bytes, 56)?,
        "trade amount mismatch"
    );
    ensure!(
        base == u64_at(bytes, 40)? && quote == u64_at(bytes, 48)?,
        "raw reserve mismatch"
    );
    let wire_virtual = if bytes.len() >= tail.checked_add(57).context("tail overflow")? {
        i128::from_le_bytes(
            bytes
                .get(tail + 32..tail + 48)
                .context("signed reserve truncated")?
                .try_into()?,
        )
    } else {
        0
    };
    ensure!(virtual_reserve == wire_virtual, "signed reserve mismatch");
    let expected = i128::from(quote)
        .checked_add(wire_virtual)
        .and_then(|n| u64::try_from(n).ok())
        .context("invalid effective reserve")?;
    ensure!(
        effective_quote_reserves(quote, virtual_reserve) == Some(expected),
        "effective reserve mismatch"
    );
    Ok(Trade {
        side,
        pool: pool.to_string(),
        base_reserve: base,
        raw_quote_reserve: quote,
        virtual_quote_reserve: virtual_reserve.to_string(),
        effective_quote_reserve: expected,
    })
}

pub fn verify(j: &Value) -> Result<Report> {
    let meta = j
        .get("meta")
        .filter(|v| v.is_object())
        .context("metadata missing")?;
    ensure!(
        meta.get("err").is_some_and(Value::is_null),
        "only successful transactions are supported"
    );
    let tx: VersionedTransaction = wincode::deserialize(
        &STANDARD.decode(
            j["transaction"][0]
                .as_str()
                .context("base64 transaction missing")?,
        )?,
    )?;
    let signature = *tx.signatures.first().context("signature missing")?;
    let slot = j["slot"].as_u64().context("slot missing")?;
    let mut keys = tx.message.static_account_keys().to_vec();
    for kind in ["writable", "readonly"] {
        for key in meta["loadedAddresses"][kind]
            .as_array()
            .context("loaded addresses missing")?
        {
            keys.push(key.as_str().context("invalid ALT key")?.parse::<Pubkey>()?);
        }
    }
    let metadata = EventMetadata {
        signature,
        slot,
        block_time_us: j["blockTime"]
            .as_i64()
            .unwrap_or_default()
            .checked_mul(1_000_000)
            .context("block time overflow")?,
        ..Default::default()
    };
    let program = pump_amm::PROGRAM_ID_PUBKEY;
    let program_name = program.to_string();
    let mut instruction_count = 0;
    let mut cpi = Vec::new();
    let mut instructions: Vec<(usize, Vec<u8>, Vec<u8>)> = tx
        .message
        .instructions()
        .iter()
        .map(|i| {
            (
                i.program_id_index as usize,
                i.accounts.clone(),
                i.data.clone(),
            )
        })
        .collect();
    for group in meta["innerInstructions"]
        .as_array()
        .context("inner instructions missing")?
    {
        for i in group["instructions"]
            .as_array()
            .context("inner group missing")?
        {
            let index: usize = i["programIdIndex"]
                .as_u64()
                .context("program index")?
                .try_into()?;
            let accounts = i["accounts"]
                .as_array()
                .context("account indices")?
                .iter()
                .map(|n| Ok(u8::try_from(n.as_u64().context("invalid index")?)?))
                .collect::<Result<Vec<_>>>()?;
            let data = bs58::decode(i["data"].as_str().context("CPI data missing")?).into_vec()?;
            instructions.push((index, accounts, data));
        }
    }
    for (index, accounts, data) in instructions {
        if keys.get(index) != Some(&program) {
            continue;
        }
        if data.starts_with(&[228, 69, 165, 46, 81, 203, 154, 29]) {
            let disc: [u8; 16] = data
                .get(..16)
                .context("CPI discriminator truncated")?
                .try_into()?;
            if supported_event(&data[8..]) {
                let mut event = pump_amm_inner::parse_pumpswap_inner_instruction(
                    &disc,
                    &data[16..],
                    metadata.clone(),
                )
                .context("supported CPI event rejected")?;
                normalize(&mut event);
                cpi.push((data[8..].to_vec(), event));
            }
            continue;
        }
        let disc: [u8; 8] = data
            .get(..8)
            .context("instruction discriminator truncated")?
            .try_into()?;
        if !matches!(
            disc,
            pump_amm::discriminators::BUY
                | pump_amm::discriminators::BUY_EXACT_QUOTE_IN
                | pump_amm::discriminators::SELL
        ) {
            continue;
        }
        let accounts = accounts
            .iter()
            .map(|index| {
                keys.get(*index as usize)
                    .copied()
                    .context("instruction account out of bounds")
            })
            .collect::<Result<Vec<_>>>()?;
        let event = pump_amm::parse_instruction(&data, &accounts, signature, slot, 0, None)
            .context("trade instruction rejected")?;
        let first = u64_at(&data, 8)?;
        let second = u64_at(&data, 16)?;
        match event {
            DexEvent::PumpSwapBuy(e) => {
                let (base, quote) = if disc == pump_amm::discriminators::BUY {
                    (first, second)
                } else {
                    (second, first)
                };
                ensure!(
                    e.base_amount_out == base && e.max_quote_amount_in == quote,
                    "buy instruction amount mismatch"
                );
                if disc == pump_amm::discriminators::BUY_EXACT_QUOTE_IN {
                    ensure!(
                        e.min_base_amount_out == second,
                        "exact quote threshold mismatch"
                    );
                }
                ensure!(
                    e.pool == accounts[0]
                        && e.user == accounts[1]
                        && e.base_mint == accounts[3]
                        && e.quote_mint == accounts[4],
                    "buy account mismatch"
                );
            }
            DexEvent::PumpSwapSell(e) => {
                ensure!(
                    e.base_amount_in == first && e.min_quote_amount_out == second,
                    "sell instruction amount mismatch"
                );
                ensure!(
                    e.pool == accounts[0]
                        && e.user == accounts[1]
                        && e.base_mint == accounts[3]
                        && e.quote_mint == accounts[4],
                    "sell account mismatch"
                );
            }
            _ => anyhow::bail!("trade instruction variant mismatch"),
        }
        instruction_count += 1;
    }
    let mut stack = Vec::new();
    let mut pairs = 0;
    let mut trades = Vec::new();
    for line in meta["logMessages"].as_array().context("logs missing")? {
        let line = line.as_str().context("invalid log")?;
        if let Some(rest) = line.strip_prefix("Program ") {
            if let Some((id, _)) = rest.split_once(" invoke [") {
                if id.parse::<Pubkey>().is_ok() {
                    stack.push(id.to_owned());
                    continue;
                }
            }
            let completed = rest
                .strip_suffix(" success")
                .or_else(|| rest.split_once(" failed: ").map(|(id, _)| id));
            if let Some(id) = completed.filter(|id| id.parse::<Pubkey>().is_ok()) {
                ensure!(
                    stack.pop().as_deref() == Some(id),
                    "unbalanced runtime logs"
                );
                continue;
            }
        }
        if stack.last().map(String::as_str) != Some(program_name.as_str()) {
            continue;
        }
        let Some(body) = line.strip_prefix("Program data: ") else {
            continue;
        };
        let wire = STANDARD.decode(body)?;
        if !supported_event(&wire) {
            continue;
        }
        let mut event = logs::parse_log(line, signature, slot, 0, Some(metadata.block_time_us), 0)
            .context("supported log event rejected")?;
        normalize(&mut event);
        let index = cpi
            .iter()
            .position(|(data, _)| *data == wire)
            .context("log event has no identical CPI event")?;
        let (_, inner) = cpi.remove(index);
        ensure!(
            serde_json::to_string(&event)? == serde_json::to_string(&inner)?,
            "log/CPI decoded fields differ"
        );
        if matches!(event, DexEvent::PumpSwapBuy(_) | DexEvent::PumpSwapSell(_)) {
            trades.push(check_trade(&event, &wire[8..])?);
        }
        pairs += 1;
    }
    ensure!(stack.is_empty(), "runtime logs truncated");
    ensure!(cpi.is_empty(), "CPI event missing from logs");
    ensure!(!trades.is_empty(), "no verified PumpSwap trade found");
    Ok(Report {
        signature: signature.to_string(),
        slot,
        version: j["version"].clone(),
        checked_instructions: instruction_count,
        checked_log_cpi_pairs: pairs,
        trades,
    })
}
