#[allow(dead_code)]
pub mod instructions;
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use sol_shred_sdk::transaction_route::{SwapProtocol, TransactionRoute};
use sol_shred_sdk::{
    analyze_yellowstone_transaction_routes, transaction::VersionedTransaction, Pubkey,
};
use std::{collections::HashMap, str::FromStr};
use yellowstone_grpc_proto::prelude as pb;

mod signed_wire;

pub fn validate(j: &Value) -> Result<(usize, TransactionRoute, usize)> {
    let tx = signed_wire::signed_transaction(j)?;
    let m = &j["meta"];
    ensure!(m.is_object(), "transaction metadata missing");
    ensure!(m.get("err").is_some(), "execution status missing");
    let addresses = |name: &str| -> Result<Vec<Pubkey>> {
        m["loadedAddresses"][name]
            .as_array()
            .context("resolved ALT addresses missing")?
            .iter()
            .map(|v| Ok(Pubkey::from_str(v.as_str().context("invalid address")?)?))
            .collect()
    };
    let writable = addresses("writable")?;
    let readonly = addresses("readonly")?;
    let mut events = vec![];
    // getTransaction does not supply a block transaction index or receive timestamp.
    sol_shred_sdk::shredstream::parse_transaction_dex_events_with_loaded_addresses(
        &tx,
        &writable,
        &readonly,
        tx.signatures[0],
        j["slot"].as_u64().context("slot missing")?,
        0,
        0,
        None,
        &mut events,
    )?;
    let mut keys = tx.message.static_account_keys().to_vec();
    keys.extend(&writable);
    keys.extend(&readonly);
    validate_outer_sells(&tx, &keys, &events)?;
    let transaction = pb::Transaction {
        signatures: tx.signatures.iter().map(|s| s.as_ref().to_vec()).collect(),
        message: Some(pb::Message {
            account_keys: tx
                .message
                .static_account_keys()
                .iter()
                .map(|p| p.to_bytes().to_vec())
                .collect(),
            instructions: tx
                .message
                .instructions()
                .iter()
                .map(|i| pb::CompiledInstruction {
                    program_id_index: i.program_id_index.into(),
                    accounts: i.accounts.clone(),
                    data: i.data.clone(),
                })
                .collect(),
            ..Default::default()
        }),
    };
    let balances = |name: &str| -> Result<Vec<pb::TokenBalance>> {
        m[name]
            .as_array()
            .context("token balances missing")?
            .iter()
            .map(|v| {
                Ok(pb::TokenBalance {
                    account_index: v["accountIndex"]
                        .as_u64()
                        .context("balance index missing")?
                        .try_into()?,
                    mint: v["mint"].as_str().context("mint missing")?.to_owned(),
                    ..Default::default()
                })
            })
            .collect()
    };
    let mut meta = pb::TransactionStatusMeta {
        loaded_writable_addresses: writable.iter().map(|p| p.to_bytes().to_vec()).collect(),
        loaded_readonly_addresses: readonly.iter().map(|p| p.to_bytes().to_vec()).collect(),
        pre_token_balances: balances("preTokenBalances")?,
        post_token_balances: balances("postTokenBalances")?,
        ..Default::default()
    };
    // The route analyzer only reads presence of err; this is not a general RPC/protobuf converter.
    if !m["err"].is_null() {
        meta.err = Some(pb::TransactionError {
            err: serde_json::to_vec(&m["err"])?,
        });
    }
    for g in m["innerInstructions"].as_array().into_iter().flatten() {
        let mut instructions = vec![];
        for i in g["instructions"]
            .as_array()
            .context("inner instructions missing")?
        {
            instructions.push(pb::InnerInstruction {
                program_id_index: i["programIdIndex"]
                    .as_u64()
                    .context("program index missing")?
                    .try_into()?,
                accounts: i["accounts"]
                    .as_array()
                    .context("accounts missing")?
                    .iter()
                    .map(|v| Ok(v.as_u64().context("invalid account index")?.try_into()?))
                    .collect::<Result<_>>()?,
                data: bs58::decode(i["data"].as_str().context("inner data missing")?).into_vec()?,
                stack_height: i["stackHeight"].as_u64().map(u32::try_from).transpose()?,
            });
        }
        meta.inner_instructions.push(pb::InnerInstructions {
            index: g["index"]
                .as_u64()
                .context("outer index missing")?
                .try_into()?,
            instructions,
        });
    }
    let route = analyze_yellowstone_transaction_routes(&transaction, &meta, &[]);
    for leg in &route.legs {
        let (data, accounts, program) = if let Some(inner) = leg.position.inner_index {
            let group = meta
                .inner_instructions
                .iter()
                .find(|g| g.index == leg.position.outer_index)
                .context("missing inner group")?;
            let i = &group.instructions[inner as usize];
            (&i.data, &i.accounts, i.program_id_index as usize)
        } else {
            let i = &transaction.message.as_ref().unwrap().instructions
                [leg.position.outer_index as usize];
            (&i.data, &i.accounts, i.program_id_index as usize)
        };
        let a = |n: usize| -> Result<Pubkey> {
            Ok(*keys
                .get(*accounts.get(n).context("account position missing")? as usize)
                .context("account out of bounds")?)
        };
        let u = |n| -> Result<u64> {
            Ok(u64::from_le_bytes(
                data.get(n..n + 8).context("amount missing")?.try_into()?,
            ))
        };
        // Independent wire assertions for protocols covered by the captured corpus.
        let (pool, trader, input, output, exact, amount, threshold) = match leg.protocol {
            SwapProtocol::RaydiumClmm => {
                ensure!(
                    data[..8] == [248, 198, 158, 145, 225, 117, 135, 200]
                        || data[..8] == [43, 4, 237, 11, 26, 201, 30, 98],
                    "unknown CLMM discriminator"
                );
                ensure!(data[40] <= 1, "invalid exact-in flag");
                (2, 0, 3, 4, data[40] == 1, u(8)?, u(16)?)
            }
            SwapProtocol::RaydiumCpmm => {
                ensure!(
                    matches!(
                        data.get(..8),
                        Some([143, 190, 90, 218, 196, 30, 51, 222])
                            | Some([55, 217, 98, 86, 163, 74, 180, 173])
                    ),
                    "unknown CPMM discriminator"
                );
                let exact = data[0] == 143;
                (
                    3,
                    0,
                    4,
                    5,
                    exact,
                    u(if exact { 8 } else { 16 })?,
                    u(if exact { 16 } else { 8 })?,
                )
            }
            SwapProtocol::PumpSwap => {
                let buy = data[..8] == [102, 6, 61, 18, 1, 218, 235, 234]
                    || data[..8] == [184, 23, 238, 97, 103, 197, 211, 61];
                let quote = data[..8] == [198, 46, 21, 82, 180, 217, 232, 112]
                    || data[..8] == [194, 171, 28, 70, 104, 77, 91, 47];
                ensure!(
                    buy || quote
                        || data[..8] == [51, 230, 133, 164, 1, 127, 131, 173]
                        || data[..8] == [93, 246, 130, 60, 231, 233, 64, 178],
                    "unknown PumpSwap discriminator"
                );
                (
                    0,
                    1,
                    if buy || quote { 6 } else { 5 },
                    if buy || quote { 5 } else { 6 },
                    !buy,
                    u(8)?,
                    u(16)?,
                )
            }
            SwapProtocol::OrcaWhirlpool => {
                let v2 = data[..8] == [43, 4, 237, 11, 26, 201, 30, 98];
                ensure!(
                    v2 || data[..8] == [248, 198, 158, 145, 225, 117, 135, 200],
                    "unknown Orca discriminator"
                );
                ensure!(data[40] <= 1 && data[41] <= 1, "invalid bool");
                let (pool, trader, x, y) = if v2 { (4, 3, 7, 9) } else { (2, 1, 3, 5) };
                (
                    pool,
                    trader,
                    if data[41] == 1 { x } else { y },
                    if data[41] == 1 { y } else { x },
                    data[40] == 1,
                    u(8)?,
                    u(16)?,
                )
            }
            SwapProtocol::MeteoraDlmm => {
                let exact = data[..8] == [248, 198, 158, 145, 225, 117, 135, 200]
                    || data[..8] == [65, 75, 63, 76, 235, 91, 91, 136];
                ensure!(
                    exact
                        || data[..8] == [250, 73, 101, 33, 38, 207, 75, 184]
                        || data[..8] == [43, 215, 247, 132, 137, 60, 243, 81],
                    "unknown DLMM discriminator"
                );
                (
                    0,
                    10,
                    4,
                    5,
                    exact,
                    u(if exact { 8 } else { 16 })?,
                    u(if exact { 16 } else { 8 })?,
                )
            }
            SwapProtocol::RaydiumAmmV4 => {
                // Official AMM V4 instruction tags: legacy 9/11; compact 16/17.
                let tag = *data.first().context("AMM tag missing")?;
                ensure!(matches!(tag, 9 | 11 | 16 | 17), "unknown AMM discriminator");
                let exact = matches!(tag, 9 | 16);
                let (trader, input, output) = if matches!(tag, 16 | 17) {
                    (7, 5, 6)
                } else if accounts.len() == 17 {
                    (16, 14, 15)
                } else {
                    (17, 15, 16)
                };
                (
                    1,
                    trader,
                    input,
                    output,
                    exact,
                    u(if exact { 1 } else { 9 })?,
                    u(if exact { 9 } else { 1 })?,
                )
            }
            SwapProtocol::LaunchLab => {
                // Current public LaunchLab IDL: payer=0, pool=4, base=5, quote=6.
                let discriminator = data.get(..8).context("LaunchLab discriminator missing")?;
                let (buy, exact) = match discriminator {
                    [250, 234, 13, 123, 213, 156, 19, 236] => (true, true),
                    [24, 211, 116, 40, 105, 3, 153, 56] => (true, false),
                    [149, 39, 222, 155, 211, 124, 152, 26] => (false, true),
                    [95, 200, 71, 34, 8, 9, 11, 166] => (false, false),
                    _ => anyhow::bail!("unknown LaunchLab discriminator"),
                };
                (
                    4,
                    0,
                    if buy { 6 } else { 5 },
                    if buy { 5 } else { 6 },
                    exact,
                    u(8)?,
                    u(16)?,
                )
            }
            _ => anyhow::bail!("add independent checks for {:?}", leg.protocol),
        };
        ensure!(
            leg.program == keys[program]
                && leg.pool == a(pool)?
                && leg.trader == a(trader)?
                && leg.input_account == a(input)?
                && leg.output_account == a(output)?,
            "swap account mismatch"
        );
        ensure!(
            leg.amount_specified_is_input == exact
                && leg.specified_amount == amount
                && leg.other_amount_threshold == threshold,
            "swap argument mismatch"
        );
        // Exercise the normal instruction-event API on the same real outer/CPI instruction.
        // Route decoding alone would not establish that event fields are correct.
        let resolved = accounts
            .iter()
            .map(|i| {
                keys.get(*i as usize)
                    .copied()
                    .context("instruction account out of bounds")
            })
            .collect::<Result<Vec<_>>>()?;
        let event = sol_shred_sdk::instr::parse_instruction_unified(
            data,
            &resolved,
            tx.signatures[0],
            j["slot"].as_u64().context("slot")?,
            0,
            None,
            0,
            None,
            &keys[program],
        )
        .context("swap instruction missing from event API")?;
        use sol_shred_sdk::core::events::DexEvent;
        let matches = match event {
            DexEvent::RaydiumCpmmSwap(e) => {
                e.pool_id == a(pool)?
                    && e.payer == a(trader)?
                    && e.input_token_account == a(input)?
                    && e.output_token_account == a(output)?
                    && e.base_input == exact
                    && if exact {
                        e.amount_in == amount && e.minimum_amount_out == threshold
                    } else {
                        e.amount_out == amount && e.max_amount_in == threshold
                    }
            }
            DexEvent::RaydiumClmmSwap(e) => {
                e.pool_state == a(pool)?
                    && e.sender == a(trader)?
                    && e.token_account_0 == a(input)?
                    && e.token_account_1 == a(output)?
                    && e.amount == amount
                    && e.other_amount_threshold == threshold
                    && e.is_base_input == exact
            }
            DexEvent::OrcaWhirlpoolSwap(e) => {
                e.whirlpool == a(pool)?
                    && e.token_authority == a(trader)?
                    && e.token_owner_account_a
                        == a(if data[..8] == [43, 4, 237, 11, 26, 201, 30, 98] {
                            7
                        } else {
                            3
                        })?
                    && e.token_owner_account_b
                        == a(if data[..8] == [43, 4, 237, 11, 26, 201, 30, 98] {
                            9
                        } else {
                            5
                        })?
                    && e.amount == amount
                    && e.other_amount_threshold == threshold
                    && e.amount_specified_is_input == exact
            }
            DexEvent::PumpSwapBuy(e) => {
                e.pool == a(pool)?
                    && e.user == a(trader)?
                    && e.user_quote_token_account == a(input)?
                    && e.user_base_token_account == a(output)?
                    && e.base_amount_out == if exact { threshold } else { amount }
                    && e.max_quote_amount_in == if exact { amount } else { threshold }
            }
            DexEvent::PumpSwapSell(e) => {
                e.pool == a(pool)?
                    && e.user == a(trader)?
                    && e.user_base_token_account == a(input)?
                    && e.user_quote_token_account == a(output)?
                    && e.base_amount_in == amount
                    && e.min_quote_amount_out == threshold
            }
            DexEvent::MeteoraDlmmSwap(e) => {
                e.pool == a(pool)?
                    && e.from == a(trader)?
                    && e.user_token_in == a(input)?
                    && e.user_token_out == a(output)?
                    && e.amount_in == u(8)?
                    && if exact {
                        e.min_amount_out == u(16)?
                    } else {
                        e.amount_out == u(16)?
                    }
            }
            DexEvent::RaydiumAmmV4Swap(e) => {
                e.amm == a(pool)?
                    && e.user_source_owner == a(trader)?
                    && e.user_source_token_account == a(input)?
                    && e.user_destination_token_account == a(output)?
                    && if exact {
                        e.instruction_amount_in == amount && e.minimum_amount_out == threshold
                    } else {
                        e.instruction_amount_out == amount && e.max_amount_in == threshold
                    }
            }
            DexEvent::RaydiumLaunchlabTrade(e) => {
                let buy = input == 6;
                e.pool_state == a(pool)?
                    && e.user == a(trader)?
                    && e.is_buy == buy
                    && e.exact_in == exact
                    && e.user_base_token == a(5)?
                    && e.user_quote_token == a(6)?
                    && e.amount_in == if exact { amount } else { threshold }
                    && e.amount_out == if exact { threshold } else { amount }
            }
            _ => false,
        };
        ensure!(
            matches,
            "instruction-event API disagrees with raw wire fields"
        );
        // Independently sum raw token CPI arguments inside this invocation's stack scope.
        // Do not infer a leg's gross amounts from a whole-transaction net balance.
        let mut debit = None::<u64>;
        let mut credit = None::<u64>;
        let mut unknown_credit_fee = false;
        if let Some(group) = meta
            .inner_instructions
            .iter()
            .find(|g| g.index == leg.position.outer_index)
        {
            let start = leg.position.inner_index.map_or(0, |i| i as usize + 1);
            for child in &group.instructions[start..] {
                if leg.position.inner_index.is_some()
                    && !matches!((leg.position.stack_height, child.stack_height), (Some(parent), Some(depth)) if depth > parent)
                {
                    break;
                }
                let program = keys
                    .get(child.program_id_index as usize)
                    .context("CPI program out of bounds")?
                    .to_string();
                let legacy = program == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
                if !legacy && program != "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb" {
                    continue;
                }
                let (dest, offset, fee) = match child.data.first() {
                    Some(3) if child.data.len() == 9 && child.accounts.len() >= 3 => {
                        (1, 1, legacy.then_some(0))
                    }
                    Some(12) if child.data.len() == 10 && child.accounts.len() >= 4 => {
                        (2, 1, legacy.then_some(0))
                    }
                    Some(26)
                        if !legacy
                            && child.data.get(1) == Some(&1)
                            && child.data.len() == 19
                            && child.accounts.len() >= 4 =>
                    {
                        (
                            2,
                            2,
                            Some(u64::from_le_bytes(child.data[11..19].try_into()?)),
                        )
                    }
                    _ => continue,
                };
                let source = *keys.get(child.accounts[0] as usize).context("CPI source")?;
                let destination = *keys
                    .get(child.accounts[dest] as usize)
                    .context("CPI destination")?;
                let amount = u64::from_le_bytes(child.data[offset..offset + 8].try_into()?);
                if source == a(input)? {
                    debit = Some(
                        debit
                            .unwrap_or(0)
                            .checked_add(amount)
                            .context("debit overflow")?,
                    );
                }
                if destination == a(output)? {
                    if let Some(fee) = fee {
                        credit = Some(
                            credit
                                .unwrap_or(0)
                                .checked_add(amount.checked_sub(fee).context("fee exceeds amount")?)
                                .context("credit overflow")?,
                        );
                    } else {
                        unknown_credit_fee = true;
                    }
                }
            }
        }
        if !route.succeeded {
            debit = None;
            credit = None;
        }
        if unknown_credit_fee {
            credit = None;
        }
        ensure!(
            leg.actual_input_amount == debit && leg.actual_output_amount == credit,
            "swap executed amounts disagree with raw CPI transfers"
        );
    }
    let mut net: HashMap<Pubkey, i128> = HashMap::new();
    let mut uncertain = std::collections::HashSet::new();
    for t in &route.transfers {
        *net.entry(t.source).or_default() -= i128::from(t.amount);
        if let Some(fee) = t.withheld_fee {
            *net.entry(t.destination).or_default() += i128::from(t.amount) - i128::from(fee);
        } else {
            uncertain.insert(t.destination);
        }
    }
    for n in &route.native_token_actions {
        uncertain.insert(n.account);
    }
    // Mint/burn and account initialization/closure require separate supply/lifecycle accounting.
    let mut mark = |program: u32, accounts: &[u8], data: &[u8]| {
        if keys.get(program as usize).is_some_and(|p| {
            matches!(
                p.to_string().as_str(),
                "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
                    | "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
            )
        }) && !matches!(data.first(), Some(3 | 12 | 26))
        {
            for a in accounts {
                if let Some(k) = keys.get(*a as usize) {
                    uncertain.insert(*k);
                }
            }
        }
    };
    for i in &transaction.message.as_ref().unwrap().instructions {
        mark(i.program_id_index, &i.accounts, &i.data);
    }
    for g in &meta.inner_instructions {
        for i in &g.instructions {
            mark(i.program_id_index, &i.accounts, &i.data);
        }
    }
    let mut checked = 0;
    if route.succeeded {
        for pre in m["preTokenBalances"].as_array().unwrap() {
            let index = pre["accountIndex"].as_u64().context("index")? as usize;
            let key = *keys.get(index).context("balance index out of bounds")?;
            if uncertain.contains(&key) {
                continue;
            }
            let Some(post) = m["postTokenBalances"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["accountIndex"] == pre["accountIndex"] && v["mint"] == pre["mint"])
            else {
                continue;
            };
            let amount = |v: &Value| -> Result<i128> {
                Ok(v["uiTokenAmount"]["amount"]
                    .as_str()
                    .context("raw balance missing")?
                    .parse()?)
            };
            if let Some(delta) = net.get(&key) {
                ensure!(
                    amount(post)? - amount(pre)? == *delta,
                    "token balance mismatch for {key}"
                );
                checked += 1;
            }
        }
    }
    Ok((events.len(), route, checked))
}

fn validate_outer_sells(
    tx: &VersionedTransaction,
    keys: &[Pubkey],
    events: &[sol_shred_sdk::DexEvent],
) -> Result<()> {
    use sol_shred_sdk::instr::pump_amm::{discriminators, PROGRAM_ID_PUBKEY};
    let instructions = tx.message.instructions();
    let mut used = vec![false; instructions.len()];
    for event in events {
        let sol_shred_sdk::DexEvent::PumpSwapSell(event) = event else {
            continue;
        };
        let matched = instructions.iter().enumerate().find(|(index, ix)| {
            let get = |position| {
                ix.accounts
                    .get(position)
                    .and_then(|index| keys.get(*index as usize))
                    .copied()
            };
            !used[*index]
                && keys.get(ix.program_id_index as usize) == Some(&PROGRAM_ID_PUBKEY)
                && ix.data.get(..8).is_some_and(|disc| {
                    disc == discriminators::SELL || disc == discriminators::SELL_V2
                })
                && get(0) == Some(event.pool)
                && get(1) == Some(event.user)
                && get(3) == Some(event.base_mint)
                && get(4) == Some(event.quote_mint)
                && ix.data.get(8..16) == Some(event.base_amount_in.to_le_bytes().as_slice())
                && ix.data.get(16..24) == Some(event.min_quote_amount_out.to_le_bytes().as_slice())
        });
        let (index, _) = matched.context("outer sell arguments mismatch or instruction missing")?;
        used[index] = true;
    }
    Ok(())
}

#[cfg(test)]
mod sell_validation_tests {
    use super::*;
    use sol_shred_sdk::{
        core::events::PumpSwapSellEvent,
        hash::Hash,
        instruction::CompiledInstruction,
        message::{v0, MessageHeader, VersionedMessage},
    };

    #[test]
    fn multiple_sells_validate_their_own_instruction_and_arguments() {
        use sol_shred_sdk::instr::pump_amm::{discriminators, PROGRAM_ID_PUBKEY};
        let mut keys: Vec<_> = (0..42).map(|_| Pubkey::new_unique()).collect();
        keys.push(PROGRAM_ID_PUBKEY);
        let mut events = vec![];
        let mut instructions = vec![];
        for (start, amount, disc) in [
            (0u8, 100u64, discriminators::SELL),
            (21, 200, discriminators::SELL_V2),
        ] {
            let count = if disc == discriminators::SELL_V2 {
                17
            } else {
                21
            };
            let mut accounts: Vec<_> = (start..start + count).collect();
            accounts[16] = 42;
            instructions.push(CompiledInstruction::new_from_raw_parts(
                42,
                [
                    disc.to_vec(),
                    amount.to_le_bytes().to_vec(),
                    1u64.to_le_bytes().to_vec(),
                ]
                .concat(),
                accounts,
            ));
            events.push(sol_shred_sdk::DexEvent::PumpSwapSell(PumpSwapSellEvent {
                pool: keys[start as usize],
                user: keys[start as usize + 1],
                base_mint: keys[start as usize + 3],
                quote_mint: keys[start as usize + 4],
                base_amount_in: amount,
                min_quote_amount_out: 1,
                ..Default::default()
            }));
        }
        let mut tx = VersionedTransaction {
            signatures: vec![],
            message: VersionedMessage::V0(v0::Message {
                header: MessageHeader::default(),
                account_keys: keys.clone(),
                recent_blockhash: Hash::default(),
                instructions,
                address_table_lookups: vec![],
            }),
        };
        assert!(validate_outer_sells(&tx, &keys, &events).is_ok());
        // One instruction cannot validate the same decoded event twice.
        events.push(events[0].clone());
        assert!(validate_outer_sells(&tx, &keys, &events).is_err());
        events.pop();
        if let VersionedMessage::V0(message) = &mut tx.message {
            message.instructions[1].data.truncate(16);
        }
        assert!(validate_outer_sells(&tx, &keys, &events).is_err());
    }
}

#[allow(dead_code)] // Not called by the single-transaction example.
pub fn replay_corpus() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mainnet");
    let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("manifest.json"))?)?;
    for row in manifest.as_array().context("invalid manifest")? {
        let j = serde_json::from_slice(&std::fs::read(
            root.join(row["file"].as_str().context("file")?),
        )?)?;
        let (outer, route, checked) = validate(&j)?;
        if let Some(expected) = row.get("expected_lp_instructions") {
            ensure!(
                serde_json::to_value(instructions::validate(&j)?)? == *expected,
                "LP/collection instruction regression against raw-validated baseline"
            );
        }
        ensure!(
            route.signature.to_string() == row["signature"].as_str().context("signature")?,
            "signature mismatch"
        );
        ensure!(j["slot"] == row["slot"], "slot mismatch");
        ensure!(route.succeeded, "fixture unexpectedly failed");
        ensure!(
            route.legs.len() == row["swap_legs"].as_u64().context("legs")? as usize,
            "swap coverage changed"
        );
        ensure!(
            outer == row["outer_events"].as_u64().context("outer")? as usize,
            "outer coverage changed"
        );
        ensure!(
            checked == row["balance_checks"].as_u64().context("balance checks")? as usize,
            "balance coverage changed"
        );
        ensure!(
            serde_json::to_value(&route.legs)? == row["expected_legs"],
            "swap regression against validated baseline"
        );
        println!(
            "{}: outer={} swaps={} balance_checks={}",
            row["file"],
            outer,
            route.legs.len(),
            checked
        );
    }
    Ok(())
}
