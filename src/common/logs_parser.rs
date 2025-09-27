use std::str::FromStr;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::common::error::{ClientError, ClientResult};
use crate::common::{
    logs_data::{DexInstruction, CreateTokenInfo, TradeInfo,BonkCreateTokenInfo, TradeRequest, TradeType}, 
    logs_filters::LogFilter
};

use solana_sdk::pubkey::Pubkey;
use solana_sdk::instruction::CompiledInstruction;
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
    // First do base64 decoding
    let decoded = BASE64.decode(data)
        .map_err(|e| ClientError::Other(format!("Failed to decode base64: {}", e)))?;
    
    // Skip prefix bytes (if any)
    let mut cursor = if decoded.len() > 8 { 8 } else { 0 };
    
    // Read name length and name
    if cursor + 4 > decoded.len() {
        return Err(ClientError::Other("Data too short for name length".to_string()));
    }
    let name_len = read_u32(&decoded[cursor..]) as usize;
    cursor += 4;
    
    if cursor + name_len > decoded.len() {
        return Err(ClientError::Other(format!("Data too short for name: need {} bytes", name_len)));
    }
    let name = String::from_utf8(decoded[cursor..cursor + name_len].to_vec())
        .map_err(|e| ClientError::Other(format!("Invalid UTF-8 in name: {}", e)))?;
    cursor += name_len;
    
    // Read symbol length and symbol
    if cursor + 4 > decoded.len() {
        return Err(ClientError::Other("Data too short for symbol length".to_string()));
    }
    let symbol_len = read_u32(&decoded[cursor..]) as usize;
    cursor += 4;
    
    if cursor + symbol_len > decoded.len() {
        return Err(ClientError::Other(format!("Data too short for symbol: need {} bytes", symbol_len)));
    }
    let symbol = String::from_utf8(decoded[cursor..cursor + symbol_len].to_vec())
        .map_err(|e| ClientError::Other(format!("Invalid UTF-8 in symbol: {}", e)))?;
    cursor += symbol_len;
    
    // Read URI length and URI
    if cursor + 4 > decoded.len() {
        return Err(ClientError::Other("Data too short for URI length".to_string()));
    }
    let uri_len = read_u32(&decoded[cursor..]) as usize;
    cursor += 4;
    
    if cursor + uri_len > decoded.len() {
        return Err(ClientError::Other(format!("Data too short for URI: need {} bytes", uri_len)));
    }
    let uri = String::from_utf8(decoded[cursor..cursor + uri_len].to_vec())
        .map_err(|e| ClientError::Other(format!("Invalid UTF-8 in uri: {}", e)))?;
    cursor += uri_len;
    
    // Make sure there is enough data to read public keys
    if cursor + 32 * 4 > decoded.len() {
        return Err(ClientError::Other("Data too short for public keys".to_string()));
    }
    
    // Parse Mint Public Key
    let mint = bs58::encode(&decoded[cursor..cursor+32]).into_string();
    cursor += 32;

    // Parse Bonding Curve Public Key
    let bonding_curve = bs58::encode(&decoded[cursor..cursor+32]).into_string();
    cursor += 32;

    // Parse User Public Key
    let user = bs58::encode(&decoded[cursor..cursor+32]).into_string();
    cursor += 32;
    // Parse Creator Public Key
    let creator = bs58::encode(&decoded[cursor..cursor+32]).into_string();
    cursor += 32;


    Ok(CreateTokenInfo {
        slot: 0,
        name,
        symbol,
        uri,
        creator: Pubkey::from_str(&creator).unwrap(),
        mint: Pubkey::from_str(&mint).unwrap(),
        bonding_curve: Pubkey::from_str(&bonding_curve).unwrap(),
        user: Pubkey::from_str(&user).unwrap(),
        unit_limit: 0,
        unit_price: 0,
        fee_merchant: "UNKNOWN".to_string(),
        fee: 0,
    })
}

fn read_u32(data: &[u8]) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&data[..4]);
    u32::from_le_bytes(bytes)
}

pub fn parse_trade_data(data: &str) -> ClientResult<TradeInfo> {
    let engine = base64::engine::general_purpose::STANDARD;
    let decoded = engine.decode(data).map_err(|e| 
        ClientError::Parse(
            "Failed to decode base64".to_string(),
            e.to_string()
        )
    )?;

    let mut cursor = 8;  // Skip prefix

    // 1. Mint (32 bytes)
    let mint = bs58::encode(&decoded[cursor..cursor + 32]).into_string();
    cursor += 32;

    // 2. Sol Amount (8 bytes)
    let sol_amount = u64::from_le_bytes(decoded[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;

    // 3. Token Amount (8 bytes)
    let token_amount = u64::from_le_bytes(decoded[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;

    // 4. Is Buy (1 byte)
    let is_buy = decoded[cursor] != 0;
    cursor += 1;

    // 5. User (32 bytes)
    let user = bs58::encode(&decoded[cursor..cursor + 32]).into_string();
    cursor += 32;

    // 6. Timestamp (8 bytes)
    let timestamp = i64::from_le_bytes(decoded[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;

    // 7. Virtual Sol Reserves (8 bytes)
    let virtual_sol_reserves = u64::from_le_bytes(decoded[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;

    // 8. Virtual Token Reserves (8 bytes)
    let virtual_token_reserves = u64::from_le_bytes(decoded[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;

    let real_sol_reserves = u64::from_le_bytes(decoded[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;

    let real_token_reserves = u64::from_le_bytes(decoded[cursor..cursor + 8].try_into().unwrap());

    Ok(TradeInfo {
        slot: 0,
        mint: Pubkey::from_str(&mint).unwrap(),
        sol_amount,
        token_amount,
        is_buy,
        user: Pubkey::from_str(&user).unwrap(),
        timestamp,
        virtual_sol_reserves,
        virtual_token_reserves,
        real_sol_reserves,
        real_token_reserves,
    })
}

fn current_timestamp_millis() -> i64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");

    duration.as_millis() as i64
}

pub fn parse_instruction_create_token_data(instruction: &CompiledInstruction, accounts: &[Pubkey]) -> ClientResult<CreateTokenInfo> {
    let data = instruction.data.clone();
    if data.len() < 55 {
        return Err(ClientError::InvalidData(format!("CREATE_TOKEN_IX 指令数据长度不足: 只有 {} 字节，需要至少55字节", data.len())));
    }
    
    let mut offset = 8; // 跳过指令前缀
    
    // 1. 解析 name
    let name_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap_or([0, 0, 0, 0])) as usize;
    offset += 4;
    if name_len > 1000 || name_len == 0 {
        println!("Debug: name_len 异常: {}", name_len);
        return Err(ClientError::InvalidData(format!("name_len 异常: {}", name_len)));
    }
    // 添加这个检查，防止越界
    if offset + name_len > data.len() {
        println!("Debug: name 数据越界: offset={}, name_len={}, data_len={}", offset, name_len, data.len());
        return Err(ClientError::InvalidData("name 数据越界".to_string()));
    }
    let name = String::from_utf8_lossy(&data[offset..offset + name_len]).to_string();
    offset += name_len;
    
    // 2. 解析 symbol
    let symbol_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap_or([0, 0, 0, 0])) as usize;
    offset += 4;
    let symbol = String::from_utf8_lossy(&data[offset..offset + symbol_len]).to_string();
    offset += symbol_len;
    
    // 3. 解析 uri
    let uri_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap_or([0, 0, 0, 0])) as usize;
    offset += 4;
    let uri = String::from_utf8_lossy(&data[offset..offset + uri_len]).to_string();
    offset += uri_len;
    if offset + 32 > data.len() {
        return Err(ClientError::InvalidData(format!("数据长度不足，无法读取creator: offset={}, data.len()={}", offset, data.len())));
    }
    // 4. 解析 creator (32字节公钥)
    let creator = Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap());
    offset += 32;
    let mint = accounts[instruction.accounts[0] as usize];
    let user = accounts[instruction.accounts[7] as usize];
    let bonding_curve= accounts[instruction.accounts[2] as usize];
    Ok(CreateTokenInfo {
        slot: 0,
        name: name.to_string(),
        symbol: symbol.to_string(),
        uri: uri.to_string(),
        creator: creator,
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

    // 解析账户索引
    let accounts_data = &instruction.accounts;
    // if accounts_data.len() < 18 {
    //     return Err(ClientError::InvalidData("账户数量不足".to_string()));
    // }

    let payer_index = accounts_data[0] as usize;
    let creator_index = accounts_data[1] as usize;
    let platform_config_index = accounts_data[3] as usize;
    let pool_state_index = accounts_data[5] as usize;
    let base_mint_index = accounts_data[6] as usize;
    let base_vault_index = accounts_data[8] as usize;
    let quote_vault_index = accounts_data[9] as usize;

    // 安全地解析账户
    let payer = if payer_index < accounts.len() { accounts[payer_index].to_string() } else { String::new() };
    let creator = if creator_index < accounts.len() { accounts[creator_index].to_string() } else { String::new() };
    let platform_config = if platform_config_index < accounts.len() { accounts[platform_config_index].to_string() } else { String::new() };
    let pool_state = if pool_state_index < accounts.len() { accounts[pool_state_index].to_string() } else { String::new() };
    let base_mint = if base_mint_index < accounts.len() { accounts[base_mint_index].to_string() } else { String::new() };
    let base_vault = if base_vault_index < accounts.len() { accounts[base_vault_index].to_string() } else { String::new() };
    let quote_vault = if quote_vault_index < accounts.len() { accounts[quote_vault_index].to_string() } else { String::new() };

    // ========== 解析指令参数 ==========
    let mut offset = 8; // 跳过 discriminator
    if offset >= instruction.data.len() {
        return Err(ClientError::InvalidData("数据长度不足，无法解析参数".to_string()));
    }
    println!("base_mint: {:?}", base_mint);

    // 解析 MintParams (symbol, name, uri)
    let (symbol, name, uri, new_offset) = parse_mint_params(&instruction.data[offset..])?;
    offset += new_offset;

    // 解析 CurveParams

    let mut virtual_quote = 30000852951.0;
    let mut virtual_base = 1073025605596382.0;
    if offset < instruction.data.len() {
        let (curve_type, total_base_sell, total_quote_fund_raising, new_offset) = parse_curve_params(&instruction.data[offset..])?;
        
        match curve_type {
            0 => { // Constant
                if total_base_sell == 793100000000000 {
                    virtual_base = 1073025605596382.0;
                } else {
                    virtual_base = (total_base_sell as f64) * 1.352951211192;
                }
                if total_quote_fund_raising == 30000852951 {
                    virtual_quote = 30000852951.0;
                } else {
                    virtual_quote = (total_quote_fund_raising as f64) *  0.35295121;
                }
                // println!("Constant曲线: TotalBaseSell={}", total_base_sell);
            }
            1 => { // Fixed
                if total_base_sell == 793100000000000 {
                    virtual_base = 1073025605596382.0;
                } else {
                    virtual_base = (total_base_sell as f64) * 1.352951211192;
                }
                if total_quote_fund_raising == 30000852951 {
                    virtual_quote = 30000852951.0;
                } else {
                    virtual_quote = (total_quote_fund_raising as f64) *  0.35295121;
                }
                
                // println!("Fixed曲线: TotalBaseSell={}", total_base_sell);
            }
            2 => { // Linear
                println!("Linear曲线暂不支持");
            }
            _ => {
                println!("未知曲线类型: {}", curve_type);
            }
        }
        offset += new_offset;
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

// 修改 parse_mint_params 函数，添加调试信息
fn parse_mint_params(data: &[u8]) -> ClientResult<(String, String, String, usize)> {
    if data.len() < 1 {
        return Err(ClientError::InvalidData("MintParams数据长度不足".to_string()));
    }

    let mut offset = 0;
    
    // 添加调试信息
    // println!("parse_mint_params: 总数据长度 {}", data.len());
    // println!("parse_mint_params: 数据前32字节 {:?}", &data[0..data.len().min(32)]);
    
    // 跳过 decimals (u8)
    offset += 1;
    // println!("跳过decimals后，offset = {}", offset);
    
    // 解析 name (string) - 字符串长度是 u32
    if offset + 3 >= data.len() {
        return Err(ClientError::InvalidData("数据长度不足，无法解析 name 长度".to_string()));
    }
    let name_len_bytes = [data[offset], data[offset+1], data[offset+2], data[offset+3]];
    let name_len = u32::from_le_bytes(name_len_bytes) as usize;
    // println!("name长度字节: {:?}, 解析出的长度: {}", name_len_bytes, name_len);
    offset += 4;
    
    if offset + name_len > data.len() {
        return Err(ClientError::InvalidData(format!("数据长度不足，无法解析 name, 需要 {} 字节，实际剩余 {} 字节", name_len, data.len()-offset)));
    }
    let name = String::from_utf8_lossy(&data[offset..offset+name_len]).to_string();
    // println!("解析到name: {}", name);
    offset += name_len;

    // 解析 symbol (string) - 字符串长度是 u32
    if offset + 3 >= data.len() {
        return Err(ClientError::InvalidData("数据长度不足，无法解析 symbol 长度".to_string()));
    }
    let symbol_len_bytes = [data[offset], data[offset+1], data[offset+2], data[offset+3]];
    let symbol_len = u32::from_le_bytes(symbol_len_bytes) as usize;
    // println!("symbol长度字节: {:?}, 解析出的长度: {}", symbol_len_bytes, symbol_len);
    offset += 4;
    
    if offset + symbol_len > data.len() {
        return Err(ClientError::InvalidData(format!("数据长度不足，无法解析 symbol, 需要 {} 字节，实际剩余 {} 字节", symbol_len, data.len()-offset)));
    }
    let symbol = String::from_utf8_lossy(&data[offset..offset+symbol_len]).to_string();
    // println!("解析到symbol: {}", symbol);
    offset += symbol_len;

    // 解析 uri (string) - 字符串长度是 u32
    if offset + 3 >= data.len() {
        return Err(ClientError::InvalidData("数据长度不足，无法解析 uri 长度".to_string()));
    }
    let uri_len_bytes = [data[offset], data[offset+1], data[offset+2], data[offset+3]];
    let uri_len = u32::from_le_bytes(uri_len_bytes) as usize;
    // println!("uri长度字节: {:?}, 解析出的长度: {}", uri_len_bytes, uri_len);
    offset += 4;
    
    if offset + uri_len > data.len() {
        return Err(ClientError::InvalidData(format!("数据长度不足，无法解析 uri, 需要 {} 字节，实际剩余 {} 字节", uri_len, data.len()-offset)));
    }
    let uri = String::from_utf8_lossy(&data[offset..offset+uri_len]).to_string();
    // println!("解析到uri: {}", uri);
    offset += uri_len;

    Ok((symbol, name, uri, offset))
}

// 修改 parse_curve_params 函数
fn parse_curve_params(data: &[u8]) -> ClientResult<(u8, u64, u64, usize)> {
    if data.len() < 1 {
        return Err(ClientError::InvalidData("CurveParams数据长度不足".to_string()));
    }

    let curve_type = data[0];
    let mut offset = 1;

    // 根据曲线类型解析数据
    match curve_type {
        0 => { // Constant - u64 + u64 + u64 + u8 = 25 bytes
            if offset + 24 >= data.len() {
                return Err(ClientError::InvalidData("数据长度不足，无法解析 constant".to_string()));
            }
            let total_base_sell = u64::from_le_bytes([
                data[offset+8], data[offset+9], data[offset+10], data[offset+11],
                data[offset+12], data[offset+13], data[offset+14], data[offset+15]
            ]) ;
            let total_quote_fund_raising = u64::from_le_bytes([
                data[offset+16], data[offset+17], data[offset+18], data[offset+19],
                data[offset+20], data[offset+21], data[offset+22], data[offset+23]
            ]);
            offset += 25;
            Ok((curve_type, total_base_sell, total_quote_fund_raising, offset))  // 返回3个值
        }
        1 => { // Fixed - u64 + u64 + u8 = 17 bytes
            if offset + 16 >= data.len() {
                return Err(ClientError::InvalidData("数据长度不足，无法解析 fixed".to_string()));
            }
            let total_base_sell = u64::from_le_bytes([
                data[offset+8], data[offset+9], data[offset+10], data[offset+11],
                data[offset+12], data[offset+13], data[offset+14], data[offset+15]
            ]) ;
            let total_quote_fund_raising = u64::from_le_bytes([
                data[offset+16], data[offset+17], data[offset+18], data[offset+19],
                data[offset+20], data[offset+21], data[offset+22], data[offset+23]
            ]);
            offset += 25;
            Ok((curve_type, total_base_sell, total_quote_fund_raising, offset))  // 返回3个值
        }
        2 => { // Linear
            println!("Linear曲线暂不支持");
            Ok((curve_type, 0, 0, offset))
        }
        _ => {
            Err(ClientError::InvalidData(format!("未知的曲线类型: {}", curve_type)))
        }
    }
}


// 在 logs_parser.rs 中添加解析函数
pub fn parse_bonk_trade_data(
    instruction: &CompiledInstruction,
    accounts: &[Pubkey],
    trade_type: TradeType,
) -> ClientResult<TradeRequest> {
    if instruction.data.len() < 8 + 8 * 3 {
        return Err(ClientError::InvalidData("数据长度不足".to_string()));
    }

    let params = &instruction.data[8..];
    let amount = u64::from_le_bytes(params[0..8].try_into().unwrap());
    
    let payer_index = instruction.accounts[0] as usize;
    let base_mint_index = instruction.accounts[9] as usize;
    
    let payer = if payer_index < accounts.len() { 
        accounts[payer_index].to_string() 
    } else { 
        String::new() 
    };
    
    let base_mint = if base_mint_index < accounts.len() { 
        accounts[base_mint_index].to_string() 
    } else { 
        String::new() 
    };

    Ok(TradeRequest {
        payer,
        base_mint,
        amount,
        trade_type,
    })
}
pub fn parse_instruction_trade_data(instruction: &CompiledInstruction, accounts: &[Pubkey], is_buy: bool) -> ClientResult<TradeInfo> {
    let data = instruction.data.clone();
    // 解析数据，如果长度不足则使用默认值
    let amount = if data.len() >= 16 {
        u64::from_le_bytes(data[8..16].try_into().unwrap())
    } else {
        0 // 默认值
    };
    
    let max_sol_cost_or_min_sol_output = if data.len() >= 24 {
        u64::from_le_bytes(data[16..24].try_into().unwrap())
    } else {
        0 // 默认值
    };
    
    // 检查账户索引是否有效
    let user = if instruction.accounts.len() > 6 {
        accounts[instruction.accounts[6] as usize]
    } else {
        Pubkey::default() // 默认公钥
    };
    
    let mint = if instruction.accounts.len() > 2 {
        accounts[instruction.accounts[2] as usize]
    } else {
        Pubkey::default() // 默认公钥
    };
    
    
    
    
    let user = accounts[instruction.accounts[6] as usize];
    let mint = accounts[instruction.accounts[2] as usize];
    
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