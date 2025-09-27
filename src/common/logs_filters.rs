use crate::common::logs_data::{DexInstruction, TradeType, TradeRequest};
use crate::common::logs_parser::{parse_create_token_data, parse_trade_data, parse_instruction_create_token_data, parse_instruction_trade_data, parse_instruction_bonk_create_token_data, parse_bonk_trade_data};
use crate::common::error::ClientResult;
pub struct LogFilter;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use bs58;
use crate::shredstream::SubscribeTransactionsResponse;
use crate::shredstream::CompiledInstruction; // 添加这一行



use solana_sdk::message::VersionedMessage;
use solana_sdk::message::Message as SolanaMessage;
use solana_sdk::message::MessageHeader as SolanaMessageHeader;
use solana_sdk::instruction::CompiledInstruction as SolanaCompiledInstruction;
use solana_sdk::hash::Hash;
use solana_sdk::signature::Signature;

use solana_sdk::transaction::VersionedTransaction;

impl LogFilter {
    const PROGRAM_ID: &'static str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
    pub const CREATE_TOKEN_IX: &[u8] = &[24, 30, 200, 40, 5, 28, 7, 119];
    pub const BUY_IX: &[u8] = &[102, 6, 61, 18, 1, 218, 235, 234];
    pub const SELL_IX: &[u8] = &[51, 230, 133, 164, 1, 127, 131, 173];
    const PROGRAM_ID_2: &'static str = "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj";
    pub const BUY_EXACT_IN: &[u8] = &[250, 234, 13, 123, 213, 156, 19, 236];
    pub const BUY_EXACT_OUT: &[u8] = &[24, 211, 116, 40, 105, 3, 153, 56];
    pub const SELL_EXACT_IN: &[u8] = &[149, 39, 222, 155, 211, 124, 152, 26];
    pub const SELL_EXACT_OUT: &[u8] = &[95, 200, 71, 34, 8, 9, 11, 166];
    pub const INITIALIZE: &[u8] = &[175, 175, 109, 31, 13, 152, 155, 237];
    pub const INITIALIZE_V2: &[u8] = &[67, 153, 175, 39, 218, 16, 38, 32];
    

    /// Parse transaction logs and return instruction type and data
    pub fn parse_compiled_instruction(
        versioned_tx: &VersionedTransaction,
        bot_wallet: Option<Pubkey>) -> ClientResult<Vec<DexInstruction>> {
        let compiled_instructions = versioned_tx.message.instructions(); 
        let accounts = versioned_tx.message.static_account_keys();
        let program_id = Pubkey::from_str(Self::PROGRAM_ID).unwrap_or_default();
        let pump_index = accounts.iter().position(|key| key == &program_id);
        let mut instructions: Vec<DexInstruction> = Vec::new();
        if let Some(index) = pump_index {
            for instruction in compiled_instructions {
                if instruction.program_id_index as usize == index {
                    let all_accounts_valid = instruction.accounts.iter()
                    .all(|&acc_idx| (acc_idx as usize) < accounts.len());
                    if !all_accounts_valid {
                        continue;
                    }
                    match instruction.data.get(0..8) {
                        // create
                        Some(Self::CREATE_TOKEN_IX) => {     
                            if let Ok(token_info) = parse_instruction_create_token_data(instruction, accounts) {
                                instructions.push(DexInstruction::CreateToken(token_info));
                            };
                        }
                        // buy
                        Some(Self::BUY_IX) if instruction.data.len() == 24 && instruction.accounts.len() >= 12 => {
                            if let Ok(trade_info) = parse_instruction_trade_data(instruction, accounts, true) {
                                if let Some(bot_wallet_pubkey) = bot_wallet {
                                    if trade_info.user.to_string() == bot_wallet_pubkey.to_string() {
                                        instructions.push(DexInstruction::BotTrade(trade_info));
                                    } else {
                                        instructions.push(DexInstruction::UserTrade(trade_info));
                                    }
                                } else {
                                    instructions.push(DexInstruction::UserTrade(trade_info));
                                }
                            };
                        }
                        // sell
                        // Some(Self::SELL_IX) if instruction.data.len() == 24 && instruction.accounts.len() >= 12 => {
                        //     if let Ok(trade_info) = parse_instruction_trade_data(instruction, accounts, false) {
                        //         if let Some(bot_wallet_pubkey) = bot_wallet {
                        //             if trade_info.user.to_string() == bot_wallet_pubkey.to_string() {
                        //                 instructions.push(DexInstruction::BotTrade(trade_info));
                        //             } else {
                        //                 instructions.push(DexInstruction::UserTrade(trade_info));
                        //             }
                        //         } else {
                        //             instructions.push(DexInstruction::UserTrade(trade_info));
                        //         }
                        //     };
                        // }
                        _ => {}
                    }
                }
            }
        }

        let program_id_2 = Pubkey::from_str(Self::PROGRAM_ID_2).unwrap_or_default();
        let pump_index_2 = accounts.iter().position(|key| key == &program_id_2);
        if let Some(index) = pump_index_2 {
            let signature = versioned_tx.signatures[0].to_string();
            // println!("发现bonk交易hash: {}", signature);
            for instruction in compiled_instructions {
                if instruction.program_id_index as usize == index {
                    let all_accounts_valid = instruction.accounts.iter()
                    .all(|&acc_idx| (acc_idx as usize) < accounts.len());
                    if !all_accounts_valid {
                        continue;
                    }

                    // 检查指令数据长度
                    if instruction.data.len() < 8 {
                        continue;
                    }

                    match instruction.data.get(0..8) {
                        Some(Self::INITIALIZE) => {
                            println!("匹配到BONK创建代币指令! signature: {}", signature);
                            match parse_instruction_bonk_create_token_data(instruction, accounts) {
                                Ok(token_info) => {
                                    instructions.push(DexInstruction::BonkCreateToken(token_info));
                                }
                                Err(e) => {
                                    println!("解析BONK创建代币失败: {:?}", e);
                                }
                            }
                        }
                        Some(Self::BUY_EXACT_IN) => {
                            if let Ok(trade_request) = parse_bonk_trade_data(instruction, accounts, TradeType::BuyExactIn) {
                                instructions.push(DexInstruction::BonkTrade(trade_request));
                            }
                        }
                        Some(Self::SELL_EXACT_IN) => {
                            if let Ok(trade_request) = parse_bonk_trade_data(instruction, accounts, TradeType::SellExactIn) {
                                instructions.push(DexInstruction::BonkTrade(trade_request));
                            }
                        }
                        Some(Self::SELL_EXACT_OUT) => {
                            if let Ok(trade_request) = parse_bonk_trade_data(instruction, accounts, TradeType::SellExactOut) {  
                                instructions.push(DexInstruction::BonkTrade(trade_request));
                            }
                        }
                        _ => {}
                    }
                    
                }
            }
        }

        Ok(instructions)
    }

    fn convert_proto_instruction(
        proto_ix: &crate::shredstream::CompiledInstruction,
    ) -> SolanaCompiledInstruction {
        SolanaCompiledInstruction {
            program_id_index: proto_ix.program_id_index as u8,
            accounts: proto_ix.accounts.iter().map(|&x| x as u8).collect(),
            data: proto_ix.data.clone(),
        }
    }
    pub fn parse_compiled_instruction_shreder(
        tx_info: &SubscribeTransactionsResponse) -> ClientResult<Vec<DexInstruction>>   {

        
            let compiled_instructions = &tx_info.transaction.as_ref().unwrap().transaction.as_ref().unwrap().message.as_ref().unwrap().instructions;
            let mut accounts = tx_info.transaction.as_ref().unwrap().transaction.as_ref().unwrap().message.as_ref().unwrap().account_keys.clone();
            let address_table_lookups = &tx_info.transaction.as_ref().unwrap().transaction.as_ref().unwrap().message.as_ref().unwrap().address_table_lookups;
        let mut all_accounts = accounts.clone();
        for lookup in address_table_lookups {
            accounts.push(lookup.account_key.clone());
        }
        // let tx_signature = bs58::encode(&tx_info.signature).into_string();
        let tx_signature = bs58::encode(&tx_info.transaction.as_ref().unwrap().transaction.as_ref().unwrap().signatures[0]).into_string();
        

        
        let program_id = Pubkey::from_str(Self::PROGRAM_ID).unwrap_or_default();
        // let pump_index = accounts.iter().position(|key| key == &program_id);
        let mut instructions: Vec<DexInstruction> = Vec::new();
        for instruction in compiled_instructions {
            let all_accounts_valid = instruction.accounts.iter()
            .all(|&acc_idx| (acc_idx as usize) < accounts.len());
            if !all_accounts_valid {
                continue;
            }
            let solana_ix = Self::convert_proto_instruction(instruction);
            // 转换账户类型
            let accounts_pubkeys: Vec<Pubkey> = accounts.iter()
            .map(|key_bytes| {
                let mut array = [0u8; 32];
                let len = std::cmp::min(key_bytes.len(), 32);
                array[..len].copy_from_slice(&key_bytes[..len]);
                Pubkey::new_from_array(array)
            })
            .collect();
            match instruction.data.get(0..8) {
                // create
                
                Some(Self::CREATE_TOKEN_IX) => { 
                    // println!("匹配到创建代币指令! signature: {}", tx_signature);
                    if let Ok(token_info) = parse_instruction_create_token_data(&solana_ix, &accounts_pubkeys) {
                        instructions.push(DexInstruction::CreateToken(token_info));
                    };
                }
                // buy
                Some(Self::BUY_IX) if instruction.accounts.len() > 7 => {
                    if let Ok(trade_info) = parse_instruction_trade_data(&solana_ix, &accounts_pubkeys, true) {
                        instructions.push(DexInstruction::UserTrade(trade_info));
                    };
                }
                // sell
                Some(Self::SELL_IX) if instruction.data.len() == 24 && instruction.accounts.len() >= 12 => {
                    if let Ok(trade_info) = parse_instruction_trade_data(&solana_ix, &accounts_pubkeys, false) {
                        instructions.push(DexInstruction::UserTrade(trade_info));
                    };
                }
                _ => {}
            }
        }


        let program_id_2 = Pubkey::from_str(Self::PROGRAM_ID_2).unwrap_or_default();
        let program_id_2_bytes = program_id_2.to_bytes().to_vec();
        let pump_index_2 = accounts.iter().position(|key| key == &program_id_2_bytes);
        if let Some(index) = pump_index_2 {
            let signature = tx_signature.clone();
            println!("发现bonk交易hash: {}", signature);
            for instruction in compiled_instructions {
                if instruction.program_id_index as usize == index {
                    let all_accounts_valid = instruction.accounts.iter()
                    .all(|&acc_idx| (acc_idx as usize) < accounts.len());
                    if !all_accounts_valid {
                        continue;
                    }

                    // 检查指令数据长度
                    if instruction.data.len() < 8 {
                        continue;
                    }
                    let solana_ix = Self::convert_proto_instruction(instruction);
                    let accounts_pubkeys: Vec<Pubkey> = accounts.iter()
                        .map(|key_bytes| {
                            let mut array = [0u8; 32];
                            let len = std::cmp::min(key_bytes.len(), 32);
                            array[..len].copy_from_slice(&key_bytes[..len]);
                            Pubkey::new_from_array(array)
                        })
                        .collect();

                    match instruction.data.get(0..8) {
                        Some(Self::INITIALIZE_V2) => {
                            println!("匹配到BONK创建代币指令! signature: {}", signature);
                            match parse_instruction_bonk_create_token_data(&solana_ix, &accounts_pubkeys) {
                                Ok(token_info) => {
                                    instructions.push(DexInstruction::BonkCreateToken(token_info));
                                }
                                Err(e) => {
                                    println!("解析BONK创建代币失败: {:?}", e);
                                }
                            }
                        }
                        Some(Self::BUY_EXACT_IN) => {
                            if let Ok(trade_request) = parse_bonk_trade_data(&solana_ix, &accounts_pubkeys, TradeType::BuyExactIn) {
                                instructions.push(DexInstruction::BonkTrade(trade_request));
                            }
                        }
                        Some(Self::SELL_EXACT_IN) => {
                            if let Ok(trade_request) = parse_bonk_trade_data(&solana_ix, &accounts_pubkeys, TradeType::SellExactIn) {
                                instructions.push(DexInstruction::BonkTrade(trade_request));
                            }
                        }
                        Some(Self::SELL_EXACT_OUT) => {
                            if let Ok(trade_request) = parse_bonk_trade_data(&solana_ix, &accounts_pubkeys, TradeType::SellExactOut) {  
                                instructions.push(DexInstruction::BonkTrade(trade_request));
                            }
                        }
                        _ => {}
                    }
                    
                }
            }
        }


        Ok(instructions)
    }


    pub fn parse_tip_info_shreder(tx_info: &SubscribeTransactionsResponse) -> (Option<u32>, Option<u64>, Option<String>, Option<u64>) {

        
        let compiled_instructions = &tx_info.transaction.as_ref().unwrap().transaction.as_ref().unwrap().message.as_ref().unwrap().instructions;
        let accounts = &tx_info.transaction.as_ref().unwrap().transaction.as_ref().unwrap().message.as_ref().unwrap().account_keys;
        let accounts_pubkeys: Vec<Pubkey> = accounts.iter()
        .map(|key_bytes| {
            let mut array = [0u8; 32];
            let len = std::cmp::min(key_bytes.len(), 32);
            array[..len].copy_from_slice(&key_bytes[..len]);
            Pubkey::new_from_array(array)
        })
        .collect();
        

        let mut unit_limit: Option<u32> = None;
        let mut unit_price: Option<u64> = None;
        let mut fee_merchant: Option<String> = None;
        let mut fee: Option<u64> = None;
        
        for proto_ix in compiled_instructions {
            
            let instruction = Self::convert_proto_instruction(proto_ix);
            let program_id = accounts_pubkeys[instruction.program_id_index as usize];
            if program_id.to_string() == "ComputeBudget111111111111111111111111111111" {
                if instruction.data.len() > 0 {
                    match instruction.data[0] {
                        2 => {
                            if instruction.data.len() >= 5 {
                                unit_limit = Some(u32::from_le_bytes([
                                    instruction.data[1], instruction.data[2], 
                                    instruction.data[3], instruction.data[4]
                                ]));
                            }
                        }
                        3 => {
                            if instruction.data.len() >= 9 {
                                unit_price = Some(u64::from_le_bytes([
                                    instruction.data[1], instruction.data[2], instruction.data[3], instruction.data[4],
                                    instruction.data[5], instruction.data[6], instruction.data[7], instruction.data[8]
                                ]));
                            }
                        }
                        _ => {}
                    }
                }
            }
            // 识别 System Program 转账
            if program_id.to_string() == "11111111111111111111111111111111"
            && instruction.data.len() >= 12
            && instruction.data[0] == 2 && instruction.data[1] == 0 && instruction.data[2] == 0 && instruction.data[3] == 0
            && instruction.accounts.len() >= 2
            {
                // 解析收款人
                let to_index = instruction.accounts[1] as usize;
                if to_index < accounts.len() {
                    let recipient = accounts_pubkeys[to_index].to_string();
                    let amount = u64::from_le_bytes([
                        instruction.data[4], instruction.data[5], instruction.data[6], instruction.data[7],
                        instruction.data[8], instruction.data[9], instruction.data[10], instruction.data[11],
                    ]);
                    let tip_type = Self::get_tip_type(&recipient);
                    fee_merchant = Some(tip_type);
                    fee = Some(amount);
                }
            }
        }
        
        (unit_limit, unit_price, fee_merchant, fee)
    }


    pub fn parse_tip_info(versioned_tx: &VersionedTransaction) -> (Option<u32>, Option<u64>, Option<String>, Option<u64>) {
        let compiled_instructions = versioned_tx.message.instructions(); 
        let accounts = versioned_tx.message.static_account_keys();
        let mut unit_limit: Option<u32> = None;
        let mut unit_price: Option<u64> = None;
        let mut fee_merchant: Option<String> = None;
        let mut fee: Option<u64> = None;
        
        for instruction in compiled_instructions {
            let program_id = accounts[instruction.program_id_index as usize];
            
            if program_id.to_string() == "ComputeBudget111111111111111111111111111111" {
                if instruction.data.len() > 0 {
                    match instruction.data[0] {
                        2 => {
                            if instruction.data.len() >= 5 {
                                unit_limit = Some(u32::from_le_bytes([
                                    instruction.data[1], instruction.data[2], 
                                    instruction.data[3], instruction.data[4]
                                ]));
                            }
                        }
                        3 => {
                            if instruction.data.len() >= 9 {
                                unit_price = Some(u64::from_le_bytes([
                                    instruction.data[1], instruction.data[2], instruction.data[3], instruction.data[4],
                                    instruction.data[5], instruction.data[6], instruction.data[7], instruction.data[8]
                                ]));
                            }
                        }
                        _ => {}
                    }
                }
            }
            // 识别 System Program 转账
            if program_id.to_string() == "11111111111111111111111111111111"
            && instruction.data.len() >= 12
            && instruction.data[0] == 2 && instruction.data[1] == 0 && instruction.data[2] == 0 && instruction.data[3] == 0
            && instruction.accounts.len() >= 2
            {
                // 解析收款人
                let to_index = instruction.accounts[1] as usize;
                if to_index < accounts.len() {
                    let recipient = accounts[to_index].to_string();
                    let amount = u64::from_le_bytes([
                        instruction.data[4], instruction.data[5], instruction.data[6], instruction.data[7],
                        instruction.data[8], instruction.data[9], instruction.data[10], instruction.data[11],
                    ]);
                    let tip_type = Self::get_tip_type(&recipient);
                    fee_merchant = Some(tip_type);
                    fee = Some(amount);
                }
            }
        }
        
        (unit_limit, unit_price, fee_merchant, fee)
    }

    // 直接在 impl LogFilter 里加这个简单函数
fn get_tip_type(recipient: &str) -> String {
    let jito_accounts = [
        "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
        "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
        "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
        "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
        "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
        "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
        "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
        "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    ];
    let solt0_accounts = [
        "DiTmWENJsHQdawVUUKnUXkconcpW4Jv52TnMWhkncF6t",
        "HRyRhQ86t3H4aAtgvHVpUJmw64BDrb61gRiKcdKUXs5c",
        "7y4whZmw388w1ggjToDLSBLv47drw5SUXcLk6jtmwixd",
        "J9BMEWFbCBEjtQ1fG5Lo9kouX1HfrKQxeUxetwXrifBw",
        "8U1JPQh3mVQ4F5jwRdFTBzvNRQaYFQppHQYoH38DJGSQ",
        "Eb2KpSC8uMt9GmzyAEm5Eb1AAAgTjRaXWFjKyFXHZxF3",
        "FCjUJZ1qozm1e8romw216qyfQMaaWKxWsuySnumVCCNe",
        "ENxTEjSQ1YabmUpXAdCgevnHQ9MHdLv8tzFiuiYJqa13",
        "6rYLG55Q9RpsPGvqdPNJs4z5WTxJVatMB8zV3WJhs5EK",
        "Cix2bHfqPcKcM233mzxbLk14kSggUUiz2A87fJtGivXr",
    ];
    if jito_accounts.contains(&recipient) {
        "JITO".to_string()
    } else if solt0_accounts.contains(&recipient) {
        "SOLT0".to_string()
    } else if recipient.starts_with("node") {
        "NODE".to_string()
    } else if recipient.starts_with("noz") || recipient.starts_with("TEMP") {
        "TEMP".to_string()
    } else if recipient.starts_with("Next") || recipient.starts_with("neXt") || recipient.starts_with("next") {
        "NEXT".to_string()
    } else {
        "UNKNOWN".to_string()
    }
}
    
    /// Parse transaction logs and return instruction type and data
    pub fn parse_instruction(logs: &[String], bot_wallet: Option<Pubkey>) -> ClientResult<Vec<DexInstruction>> {
        let mut current_instruction = None;
        let mut program_data = String::new();
        let mut invoke_depth = 0;
        let mut last_data_len = 0;
        let mut instructions = Vec::new();
        for log in logs {
            // Check program invocation
            if log.contains(&format!("Program {} invoke", Self::PROGRAM_ID)) {
                invoke_depth += 1;
                if invoke_depth == 1 {  // Only reset state at top level call
                    current_instruction = None;
                    program_data.clear();
                    last_data_len = 0;
                }
                continue;
            }
            
            // Skip if not in our program
            if invoke_depth == 0 {
                continue;
            }
            
            // Identify instruction type (only at top level)
            if invoke_depth == 1 && log.contains("Program log: Instruction:") {
                if log.contains("Create") {
                    current_instruction = Some("create");
                } else if log.contains("Buy") || log.contains("Sell") {
                    current_instruction = Some("trade");
                }
                continue;
            }
            
            // Collect Program data
            if log.starts_with("Program data: ") {
                let data = log.trim_start_matches("Program data: ");
                if data.len() > last_data_len {
                    program_data = data.to_string();
                    last_data_len = data.len();
                }
            }
            
            // Check if program ends
            if log.contains(&format!("Program {} success", Self::PROGRAM_ID)) {
                invoke_depth -= 1;
                if invoke_depth == 0 {  // Only process data when top level program ends
                    if let Some(instruction_type) = current_instruction {
                        if !program_data.is_empty() {
                            match instruction_type {
                                "create" => {
                                    if let Ok(token_info) = parse_create_token_data(&program_data) {
                                        instructions.push(DexInstruction::CreateToken(token_info));
                                    }
                                },
                                "trade" => {
                                    if let Ok(trade_info) = parse_trade_data(&program_data) {
                                        if let Some(bot_wallet_pubkey) = bot_wallet {
                                            if trade_info.user.to_string() == bot_wallet_pubkey.to_string() {
                                                instructions.push(DexInstruction::BotTrade(trade_info));
                                            } else {
                                                instructions.push(DexInstruction::UserTrade(trade_info));
                                            }
                                        } else {
                                            instructions.push(DexInstruction::UserTrade(trade_info));
                                        }
                                    }
                                },
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        Ok(instructions)
    }
}