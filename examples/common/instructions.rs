//! Targeted LP and CPMM creator-fee instruction checks against raw RPC wire data.
#[path = "signed_wire.rs"]
mod signed_wire;
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sol_shred_sdk::Pubkey;
use std::str::FromStr;

#[derive(Clone, Debug)]
struct WireInstruction {
    outer: usize,
    inner: Option<usize>,
    program: usize,
    accounts: Vec<u8>,
    data: Vec<u8>,
}

pub fn validate(j: &Value) -> Result<Vec<Value>> {
    let tx = signed_wire::signed_transaction(j)?;
    ensure!(j["meta"].get("err").is_some(), "execution status missing");
    let mut keys = tx.message.static_account_keys().to_vec();
    for name in ["writable", "readonly"] {
        for v in j["meta"]["loadedAddresses"][name]
            .as_array()
            .context("ALT addresses")?
        {
            keys.push(Pubkey::from_str(v.as_str().context("address")?)?);
        }
    }
    let mut instructions = vec![];
    for (outer, i) in tx.message.instructions().iter().enumerate() {
        instructions.push(WireInstruction {
            outer,
            inner: None,
            program: i.program_id_index as usize,
            accounts: i.accounts.clone(),
            data: i.data.clone(),
        });
        for g in j["meta"]["innerInstructions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|g| g["index"].as_u64() == Some(outer as u64))
        {
            for (inner, i) in g["instructions"]
                .as_array()
                .context("inner instructions")?
                .iter()
                .enumerate()
            {
                instructions.push(WireInstruction {
                    outer,
                    inner: Some(inner),
                    program: i["programIdIndex"]
                        .as_u64()
                        .context("program index")?
                        .try_into()?,
                    accounts: i["accounts"]
                        .as_array()
                        .context("accounts")?
                        .iter()
                        .map(|v| Ok(v.as_u64().context("account index")?.try_into()?))
                        .collect::<Result<_>>()?,
                    data: bs58::decode(i["data"].as_str().context("data")?).into_vec()?,
                });
            }
        }
    }
    let mut checked = vec![];
    for i in instructions {
        let program = *keys.get(i.program).context("program index out of bounds")?;
        let Some(disc) = i.data.get(..8) else {
            continue;
        };
        let p = program.to_string();
        let deposit = disc == [242, 35, 198, 137, 82, 225, 242, 182];
        let withdraw = disc == [183, 18, 70, 156, 148, 109, 161, 34];
        let collect = disc == [20, 22, 86, 123, 198, 28, 219, 132];
        let permissionless = disc == [202, 202, 34, 83, 226, 122, 145, 229];
        let orca_inc = disc == [46, 156, 243, 118, 13, 205, 251, 178]
            || disc == [133, 29, 89, 223, 69, 238, 176, 10];
        let orca_dec = disc == [160, 38, 208, 111, 104, 91, 44, 1]
            || disc == [58, 127, 188, 62, 79, 82, 196, 96];
        let kind = match p.as_str() {
            "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C" if deposit => "RaydiumCpmmDeposit",
            "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C" if withdraw => "RaydiumCpmmWithdraw",
            "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C" if collect || permissionless => {
                "RaydiumCpmmCollectCreatorFee"
            }
            "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA" if deposit => "PumpSwapLiquidityAdded",
            "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA" if withdraw => "PumpSwapLiquidityRemoved",
            "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc" if orca_inc => {
                "OrcaWhirlpoolLiquidityIncreased"
            }
            "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc" if orca_dec => {
                "OrcaWhirlpoolLiquidityDecreased"
            }
            _ => continue,
        };
        let accounts = i
            .accounts
            .iter()
            .map(|n| {
                keys.get(*n as usize)
                    .copied()
                    .context("account out of bounds")
            })
            .collect::<Result<Vec<_>>>()?;
        let event = sol_shred_sdk::instr::parse_instruction_unified(
            &i.data,
            &accounts,
            tx.signatures[0],
            j["slot"].as_u64().context("slot")?,
            0,
            None,
            0,
            None,
            &program,
        )
        .context("supported instruction did not parse")?;
        let event = serde_json::to_value(event)?;
        let parsed = event.get(kind).context("wrong parsed event kind")?;
        let mut args = serde_json::Map::new();
        let mut mapped = serde_json::Map::new();
        let mut check_key = |field: &str, index: usize| -> Result<()> {
            let key = accounts
                .get(index)
                .context("required account position missing")?;
            ensure!(
                parsed[field] == serde_json::to_value(key)?,
                "{kind}.{field}: account mapping mismatch"
            );
            mapped.insert(field.to_owned(), key.to_string().into());
            Ok(())
        };
        let mut check_u64 = |field: &str, offset: usize| -> Result<()> {
            let amount = u64::from_le_bytes(
                i.data
                    .get(offset..offset + 8)
                    .context("truncated u64")?
                    .try_into()?,
            );
            ensure!(
                parsed[field].as_u64() == Some(amount),
                "{kind}.{field}: amount mismatch"
            );
            args.insert(field.to_owned(), amount.into());
            Ok(())
        };
        match kind {
            "RaydiumCpmmDeposit" | "RaydiumCpmmWithdraw" => {
                check_key("pool", 2)?;
                check_key("user", 0)?;
                check_u64("lp_token_amount", 8)?;
                check_u64("token0_amount", 16)?;
                check_u64("token1_amount", 24)?;
            }
            "PumpSwapLiquidityAdded" | "PumpSwapLiquidityRemoved" => {
                check_key("pool", 0)?;
                check_key("user", 2)?;
                check_key("user_base_token_account", 6)?;
                check_key("user_quote_token_account", 7)?;
                check_key("user_pool_token_account", 8)?;
                let fields = if deposit {
                    [
                        "lp_token_amount_out",
                        "max_base_amount_in",
                        "max_quote_amount_in",
                    ]
                } else {
                    [
                        "lp_token_amount_in",
                        "min_base_amount_out",
                        "min_quote_amount_out",
                    ]
                };
                for (n, field) in fields.iter().enumerate() {
                    check_u64(field, 8 + 8 * n)?;
                }
            }
            "OrcaWhirlpoolLiquidityIncreased" | "OrcaWhirlpoolLiquidityDecreased" => {
                let v2 = disc == [133, 29, 89, 223, 69, 238, 176, 10]
                    || disc == [58, 127, 188, 62, 79, 82, 196, 96];
                check_key("whirlpool", 0)?;
                check_key("position", if v2 { 5 } else { 3 })?;
                let liquidity = u128::from_le_bytes(
                    i.data
                        .get(8..24)
                        .context("truncated liquidity")?
                        .try_into()?,
                );
                ensure!(
                    parsed["liquidity"] == serde_json::to_value(liquidity)?,
                    "liquidity mismatch"
                );
                // Decimal string retains full u128 precision in JSON tooling.
                check_u64("token_a_amount", 24)?;
                check_u64("token_b_amount", 32)?;
                args.insert("liquidity".into(), liquidity.to_string().into());
            }
            "RaydiumCpmmCollectCreatorFee" => {
                ensure!(
                    parsed["permissionless"] == permissionless,
                    "permissionless mismatch"
                );
                check_key("payer", 0)?;
                check_key("creator", usize::from(permissionless))?;
                check_key("authority", if permissionless { 2 } else { 1 })?;
                check_key("pool_state", if permissionless { 3 } else { 2 })?;
                check_key("amm_config", if permissionless { 14 } else { 3 })?;
                check_key("creator_fee_share", if permissionless { 15 } else { 14 })?;
                for (n, field) in [
                    "token_0_vault",
                    "token_1_vault",
                    "vault_0_mint",
                    "vault_1_mint",
                    "creator_token_0",
                    "creator_token_1",
                    "token_0_program",
                    "token_1_program",
                    "associated_token_program",
                    "system_program",
                ]
                .iter()
                .enumerate()
                {
                    check_key(field, n + 4)?;
                }
                let creator = accounts[usize::from(permissionless)];
                let config = accounts[if permissionless { 14 } else { 3 }];
                let expected = Pubkey::find_program_address(
                    &[b"creator_fee_share", creator.as_ref(), config.as_ref()],
                    &program,
                )
                .0;
                ensure!(
                    accounts[if permissionless { 15 } else { 14 }] == expected,
                    "creator_fee_share PDA mismatch"
                );
                args.insert("permissionless".into(), permissionless.into());
            }
            _ => unreachable!(),
        }
        if matches!(kind, "RaydiumCpmmDeposit" | "RaydiumCpmmWithdraw")
            && j["meta"]["err"].is_null()
        {
            let group = j["meta"]["innerInstructions"]
                .as_array()
                .context("LP CPI metadata missing")?
                .iter()
                .find(|g| g["index"].as_u64() == Some(i.outer as u64))
                .context("LP CPI group missing")?;
            let children = group["instructions"]
                .as_array()
                .context("LP CPI instructions")?;
            let start = i.inner.map_or(0, |n| n + 1);
            let parent_height = i
                .inner
                .and_then(|n| children.get(n))
                .and_then(|v| v["stackHeight"].as_u64());
            let mut amount = None::<u64>;
            for child in &children[start..] {
                if i.inner.is_some()
                    && !matches!((parent_height,child["stackHeight"].as_u64()),(Some(parent),Some(depth)) if depth>parent)
                {
                    break;
                }
                let program_index =
                    child["programIdIndex"].as_u64().context("LP CPI program")? as usize;
                let token = keys
                    .get(program_index)
                    .context("LP CPI program out of bounds")?
                    .to_string();
                if token != "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
                    && token != "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
                {
                    continue;
                }
                let data =
                    bs58::decode(child["data"].as_str().context("LP CPI data")?).into_vec()?;
                let minting = kind == "RaydiumCpmmDeposit";
                if !matches!(data.first(), Some(tag) if if minting { *tag==7 || *tag==14 } else { *tag==8 || *tag==15 })
                {
                    continue;
                }
                let ac = child["accounts"].as_array().context("LP CPI accounts")?;
                let key = |n: usize| -> Result<Pubkey> {
                    Ok(*keys
                        .get(
                            ac.get(n)
                                .and_then(Value::as_u64)
                                .context("LP CPI account")? as usize,
                        )
                        .context("LP CPI account out of bounds")?)
                };
                if key(if minting { 0 } else { 1 })? != accounts[12]
                    || key(if minting { 1 } else { 0 })? != accounts[3]
                {
                    continue;
                }
                let n = u64::from_le_bytes(data.get(1..9).context("LP CPI amount")?.try_into()?);
                amount = Some(
                    amount
                        .unwrap_or(0)
                        .checked_add(n)
                        .context("LP CPI amount overflow")?,
                );
            }
            let expected = u64::from_le_bytes(i.data[8..16].try_into()?);
            ensure!(
                amount == Some(expected),
                "LP mint/burn execution differs from lp_token_amount or evidence is missing"
            );
            args.insert("executed_lp_token_amount".into(), expected.into());
        }
        checked.push(json!({"kind":kind,"outer_index":i.outer,"inner_index":i.inner,"program":p,"arguments":args,"accounts":mapped}));
    }
    Ok(checked)
}
