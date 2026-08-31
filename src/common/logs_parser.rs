use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

use crate::common::error::{ClientError, ClientResult};
use crate::common::{
    logs_data::{
        BonkCreateTokenInfo, CreateTokenInfo, DexInstruction, TradeInfo, TradeRequest, TradeType,
    },
    logs_filters::LogFilter,
};

use solana_sdk::instruction::CompiledInstruction;
use solana_sdk::pubkey::Pubkey;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn process_logs<F>(
    signature: &str,
    logs: Vec<String>,
    callback: F,
    payer: Option<Pubkey>,
) -> ClientResult<()>
where
    F: Fn(&str, DexInstruction) + Send + Sync,
{
    let instructions = LogFilter::parse_instruction(&logs, payer)?;
    for instruction in instructions {
        callback(signature, instruction);
    }
    Ok(())
}

// Add parsing function
pub fn parse_create_token_data(data: &str) -> ClientResult<CreateTokenInfo> {
    let decoded = BASE64
        .decode(data)
        .map_err(|e| ClientError::Other(format!("Failed to decode base64: {}", e)))?;

    let mut cursor = if decoded.len() > 8 { 8 } else { 0 };
    let name = read_borsh_string(&decoded, &mut cursor, "name")?;
    let symbol = read_borsh_string(&decoded, &mut cursor, "symbol")?;
    let uri = read_borsh_string(&decoded, &mut cursor, "uri")?;
    let mint = read_pubkey(&decoded, &mut cursor, "mint")?;
    let bonding_curve = read_pubkey(&decoded, &mut cursor, "bonding curve")?;
    let user = read_pubkey(&decoded, &mut cursor, "user")?;
    let creator = read_pubkey(&decoded, &mut cursor, "creator")?;

    Ok(CreateTokenInfo {
        slot: 0,
        name,
        symbol,
        uri,
        creator,
        mint,
        bonding_curve,
        user,
        unit_limit: 0,
        unit_price: 0,
        fee_merchant: "UNKNOWN".to_string(),
        fee: 0,
    })
}

pub fn parse_trade_data(data: &str) -> ClientResult<TradeInfo> {
    let engine = base64::engine::general_purpose::STANDARD;
    let decoded = engine
        .decode(data)
        .map_err(|e| ClientError::Parse("Failed to decode base64".to_string(), e.to_string()))?;

    const TRADE_DATA_LEN: usize = 8 + 32 + 8 + 8 + 1 + 32 + 8 * 5;
    if decoded.len() < TRADE_DATA_LEN {
        return Err(ClientError::InvalidData(format!(
            "trade data too short: got {}, need at least {TRADE_DATA_LEN}",
            decoded.len()
        )));
    }

    let mut cursor = 8;

    let mint = Pubkey::new_from_array(
        decoded[cursor..cursor + 32]
            .try_into()
            .map_err(|_| ClientError::InvalidData("invalid mint public key length".to_string()))?,
    );
    cursor += 32;

    // 2. Sol Amount (8 bytes)
    let sol_amount = read_u64_le(&decoded, cursor, "SOL amount")?;
    cursor += 8;

    // 3. Token Amount (8 bytes)
    let token_amount = read_u64_le(&decoded, cursor, "token amount")?;
    cursor += 8;

    // 4. Is Buy (1 byte)
    let is_buy = decoded[cursor] != 0;
    cursor += 1;

    let user = Pubkey::new_from_array(
        decoded[cursor..cursor + 32]
            .try_into()
            .map_err(|_| ClientError::InvalidData("invalid user public key length".to_string()))?,
    );
    cursor += 32;

    // 6. Timestamp (8 bytes)
    let timestamp = read_i64_le(&decoded, cursor, "timestamp")?;
    cursor += 8;

    // 7. Virtual Sol Reserves (8 bytes)
    let virtual_sol_reserves = read_u64_le(&decoded, cursor, "virtual SOL reserves")?;
    cursor += 8;

    // 8. Virtual Token Reserves (8 bytes)
    let virtual_token_reserves = read_u64_le(&decoded, cursor, "virtual token reserves")?;
    cursor += 8;

    let real_sol_reserves = read_u64_le(&decoded, cursor, "real SOL reserves")?;
    cursor += 8;

    let real_token_reserves = read_u64_le(&decoded, cursor, "real token reserves")?;

    Ok(TradeInfo {
        slot: 0,
        mint,
        sol_amount,
        token_amount,
        is_buy,
        user,
        timestamp,
        virtual_sol_reserves,
        virtual_token_reserves,
        real_sol_reserves,
        real_token_reserves,
    })
}

fn current_timestamp_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

fn read_array<const N: usize>(data: &[u8], offset: usize, field: &str) -> ClientResult<[u8; N]> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| ClientError::InvalidData(format!("{field} offset overflow")))?;
    data.get(offset..end)
        .ok_or_else(|| ClientError::InvalidData(format!("data too short for {field}")))?
        .try_into()
        .map_err(|_| ClientError::InvalidData(format!("invalid {field} length")))
}

fn read_u64_le(data: &[u8], offset: usize, field: &str) -> ClientResult<u64> {
    Ok(u64::from_le_bytes(read_array(data, offset, field)?))
}

fn read_i64_le(data: &[u8], offset: usize, field: &str) -> ClientResult<i64> {
    Ok(i64::from_le_bytes(read_array(data, offset, field)?))
}

fn read_pubkey(data: &[u8], offset: &mut usize, field: &str) -> ClientResult<Pubkey> {
    let bytes = read_array(data, *offset, field)?;
    *offset = offset
        .checked_add(32)
        .ok_or_else(|| ClientError::InvalidData(format!("{field} offset overflow")))?;
    Ok(Pubkey::new_from_array(bytes))
}

fn instruction_account(
    instruction: &CompiledInstruction,
    accounts: &[Pubkey],
    position: usize,
) -> ClientResult<Pubkey> {
    let account_index = instruction.accounts.get(position).copied().ok_or_else(|| {
        ClientError::InvalidData(format!("instruction account {position} is missing"))
    })?;
    accounts
        .get(usize::from(account_index))
        .copied()
        .ok_or_else(|| {
            ClientError::InvalidData(format!(
                "instruction account index {account_index} is out of bounds"
            ))
        })
}

fn read_borsh_string(data: &[u8], offset: &mut usize, field: &str) -> ClientResult<String> {
    let length_end = offset
        .checked_add(4)
        .ok_or_else(|| ClientError::InvalidData(format!("{field} length offset overflow")))?;
    let length_bytes = data
        .get(*offset..length_end)
        .ok_or_else(|| ClientError::InvalidData(format!("data too short for {field} length")))?;
    let length = u32::from_le_bytes(
        length_bytes
            .try_into()
            .map_err(|_| ClientError::InvalidData(format!("invalid {field} length encoding")))?,
    ) as usize;
    *offset = length_end;

    let value_end = offset
        .checked_add(length)
        .ok_or_else(|| ClientError::InvalidData(format!("{field} length overflow")))?;
    let value = data.get(*offset..value_end).ok_or_else(|| {
        ClientError::InvalidData(format!("data too short for {field}: need {length} bytes"))
    })?;
    *offset = value_end;
    String::from_utf8(value.to_vec())
        .map_err(|error| ClientError::InvalidData(format!("invalid UTF-8 in {field}: {error}")))
}

pub fn parse_instruction_create_token_data(
    instruction: &CompiledInstruction,
    accounts: &[Pubkey],
) -> ClientResult<CreateTokenInfo> {
    let data = &instruction.data;
    let mut offset = 8; // 跳过指令前缀

    let name = read_borsh_string(data, &mut offset, "name")?;
    let symbol = read_borsh_string(data, &mut offset, "symbol")?;
    let uri = read_borsh_string(data, &mut offset, "uri")?;
    let creator_end = offset
        .checked_add(32)
        .ok_or_else(|| ClientError::InvalidData("creator offset overflow".to_string()))?;
    let creator = Pubkey::new_from_array(
        data.get(offset..creator_end)
            .ok_or_else(|| ClientError::InvalidData("data too short for creator".to_string()))?
            .try_into()
            .map_err(|_| ClientError::InvalidData("invalid creator length".to_string()))?,
    );
    let mint = instruction_account(instruction, accounts, 0)?;
    let user = instruction_account(instruction, accounts, 7)?;
    let bonding_curve = instruction_account(instruction, accounts, 2)?;
    Ok(CreateTokenInfo {
        slot: 0,
        name,
        symbol,
        uri,
        creator,
        mint,
        bonding_curve,
        user,
        unit_limit: 0,
        unit_price: 0,
        fee_merchant: "UNKNOWN".to_string(),
        fee: 0,
    })
}

pub fn parse_instruction_bonk_create_token_data(
    instruction: &CompiledInstruction,
    accounts: &[Pubkey],
) -> ClientResult<BonkCreateTokenInfo> {
    // 检查指令数据长度
    if instruction.data.len() < 8 {
        return Err(ClientError::InvalidData("指令数据长度不足".to_string()));
    }

    let accounts_data = &instruction.accounts;
    if accounts_data.len() <= 9 {
        return Err(ClientError::InvalidData(format!(
            "Bonk create instruction has {} accounts, need at least 10",
            accounts_data.len()
        )));
    }

    let payer_index = accounts_data[0] as usize;
    let creator_index = accounts_data[1] as usize;
    let platform_config_index = accounts_data[3] as usize;
    let pool_state_index = accounts_data[5] as usize;
    let base_mint_index = accounts_data[6] as usize;
    let base_vault_index = accounts_data[8] as usize;
    let quote_vault_index = accounts_data[9] as usize;

    let account_string = |index: usize, field: &str| {
        accounts.get(index).map(ToString::to_string).ok_or_else(|| {
            ClientError::InvalidData(format!("{field} account index {index} is out of bounds"))
        })
    };
    let payer = account_string(payer_index, "payer")?;
    let creator = account_string(creator_index, "creator")?;
    let platform_config = account_string(platform_config_index, "platform config")?;
    let pool_state = account_string(pool_state_index, "pool state")?;
    let base_mint = account_string(base_mint_index, "base mint")?;
    let base_vault = account_string(base_vault_index, "base vault")?;
    let quote_vault = account_string(quote_vault_index, "quote vault")?;

    // ========== 解析指令参数 ==========
    let mut offset = 8; // 跳过 discriminator
    if offset >= instruction.data.len() {
        return Err(ClientError::InvalidData(
            "数据长度不足，无法解析参数".to_string(),
        ));
    }
    // 解析 MintParams (symbol, name, uri)
    let (symbol, name, uri, new_offset) = parse_mint_params(&instruction.data[offset..])?;
    offset += new_offset;

    // 解析 CurveParams

    let mut virtual_quote = 30000852951.0;
    let mut virtual_base = 1073025605596382.0;
    if offset < instruction.data.len() {
        let (curve_type, total_base_sell, total_quote_fund_raising, _new_offset) =
            parse_curve_params(&instruction.data[offset..])?;

        match curve_type {
            0 | 1 => {
                if total_base_sell == 793100000000000 {
                    virtual_base = 1073025605596382.0;
                } else {
                    virtual_base = (total_base_sell as f64) * 1.352951211192;
                }
                if total_quote_fund_raising == 30000852951 {
                    virtual_quote = 30000852951.0;
                } else {
                    virtual_quote = (total_quote_fund_raising as f64) * 0.35295121;
                }
            }
            2 => {}
            _ => {}
        }
    }

    Ok(BonkCreateTokenInfo {
        payer,
        creator,
        base_mint,
        pool_state,
        platform_config,
        virtual_base,
        virtual_quote,
        base_vault,
        quote_vault,
        symbol,
        name,
        uri,
        unit_limit: 0,
        unit_price: 0,
        fee_merchant: "UNKNOWN".to_string(),
        fee: 0,
    })
}

fn parse_mint_params(data: &[u8]) -> ClientResult<(String, String, String, usize)> {
    data.first()
        .ok_or_else(|| ClientError::InvalidData("MintParams data is empty".to_string()))?;
    let mut offset = 1;
    let name = read_borsh_string(data, &mut offset, "name")?;
    let symbol = read_borsh_string(data, &mut offset, "symbol")?;
    let uri = read_borsh_string(data, &mut offset, "uri")?;

    Ok((symbol, name, uri, offset))
}

fn parse_curve_params(data: &[u8]) -> ClientResult<(u8, u64, u64, usize)> {
    if data.is_empty() {
        return Err(ClientError::InvalidData(
            "CurveParams数据长度不足".to_string(),
        ));
    }

    let curve_type = data[0];
    let parse_reserves = |curve_name: &str| {
        let params = data.get(1..26).ok_or_else(|| {
            ClientError::InvalidData(format!("data too short to parse {curve_name} curve"))
        })?;
        let total_base_sell = read_u64_le(params, 8, "curve base reserve")?;
        let total_quote_fund_raising = read_u64_le(params, 16, "curve quote reserve")?;
        Ok((total_base_sell, total_quote_fund_raising))
    };

    match curve_type {
        0 | 1 => {
            let curve_name = if curve_type == 0 { "constant" } else { "fixed" };
            let (total_base_sell, total_quote_fund_raising) = parse_reserves(curve_name)?;
            Ok((curve_type, total_base_sell, total_quote_fund_raising, 26))
        }
        2 => Ok((curve_type, 0, 0, 1)),
        _ => Err(ClientError::InvalidData(format!(
            "未知的曲线类型: {}",
            curve_type
        ))),
    }
}

pub fn parse_bonk_trade_data(
    instruction: &CompiledInstruction,
    accounts: &[Pubkey],
    trade_type: TradeType,
) -> ClientResult<TradeRequest> {
    if instruction.data.len() < 8 + 8 * 3 {
        return Err(ClientError::InvalidData("数据长度不足".to_string()));
    }

    let amount = read_u64_le(&instruction.data, 8, "trade amount")?;

    let payer = instruction_account(instruction, accounts, 0)?.to_string();
    let base_mint = instruction_account(instruction, accounts, 9)?.to_string();

    Ok(TradeRequest {
        payer,
        base_mint,
        amount,
        trade_type,
    })
}
pub fn parse_instruction_trade_data(
    instruction: &CompiledInstruction,
    accounts: &[Pubkey],
    is_buy: bool,
) -> ClientResult<TradeInfo> {
    let data = &instruction.data;
    if data.len() < 24 {
        return Err(ClientError::InvalidData(format!(
            "trade instruction data too short: got {}, need at least 24",
            data.len()
        )));
    }
    let amount = read_u64_le(data, 8, "trade amount")?;
    let max_sol_cost_or_min_sol_output = read_u64_le(data, 16, "trade quote amount")?;

    let user = instruction_account(instruction, accounts, 6)?;
    let mint = instruction_account(instruction, accounts, 2)?;

    Ok(TradeInfo {
        slot: 0,
        mint,
        sol_amount: max_sol_cost_or_min_sol_output,
        token_amount: amount,
        is_buy,
        user,
        timestamp: current_timestamp_millis(),
        virtual_sol_reserves: 0,
        virtual_token_reserves: 0,
        real_sol_reserves: 0,
        real_token_reserves: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_borsh_string(buffer: &mut Vec<u8>, value: &str) {
        buffer.extend_from_slice(&(value.len() as u32).to_le_bytes());
        buffer.extend_from_slice(value.as_bytes());
    }

    #[test]
    fn short_trade_log_data_returns_error() {
        let encoded = BASE64.encode([0u8; 32]);
        assert!(parse_trade_data(&encoded).is_err());
    }

    #[test]
    fn truncated_create_strings_return_error() {
        let mut data = vec![0u8; 8];
        push_borsh_string(&mut data, "name");
        data.extend_from_slice(&10u32.to_le_bytes());
        data.extend_from_slice(b"x");

        let instruction = CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data,
        };
        assert!(parse_instruction_create_token_data(&instruction, &[]).is_err());
    }

    #[test]
    fn create_instruction_rejects_missing_accounts() {
        let mut data = vec![0u8; 8];
        push_borsh_string(&mut data, "name");
        push_borsh_string(&mut data, "SYM");
        push_borsh_string(&mut data, "https://example.invalid");
        data.extend_from_slice(Pubkey::new_unique().as_ref());

        let instruction = CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data,
        };
        assert!(parse_instruction_create_token_data(&instruction, &[]).is_err());
    }

    #[test]
    fn fixed_curve_rejects_truncated_reserves() {
        assert!(parse_curve_params(&[1u8; 25]).is_err());
    }

    #[test]
    fn fixed_curve_reads_reserves_at_protocol_offsets() {
        let mut data = [0u8; 26];
        data[0] = 1;
        data[9..17].copy_from_slice(&123u64.to_le_bytes());
        data[17..25].copy_from_slice(&456u64.to_le_bytes());

        let parsed = parse_curve_params(&data).expect("valid fixed curve");
        assert_eq!(parsed, (1, 123, 456, 26));
    }

    #[test]
    fn bonk_trade_rejects_short_account_list() {
        let instruction = CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: vec![0u8; 32],
        };
        assert!(parse_bonk_trade_data(&instruction, &[], TradeType::BuyExactIn).is_err());
    }
}
