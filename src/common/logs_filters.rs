use crate::common::error::{ClientError, ClientResult};
use crate::common::logs_data::{DexInstruction, TradeType};
use crate::common::logs_parser::{
    parse_bonk_trade_data, parse_create_token_data, parse_instruction_bonk_create_token_data,
    parse_instruction_create_token_data, parse_instruction_trade_data, parse_trade_data,
};
use crate::instr::program_ids::{PUMPFUN_PROGRAM_ID, RAYDIUM_LAUNCHLAB_PROGRAM_ID};
pub struct LogFilter;
use crate::shredstream::SubscribeTransactionsResponse;
use solana_sdk::instruction::CompiledInstruction as SolanaCompiledInstruction;
use solana_sdk::pubkey::Pubkey;

use solana_sdk::transaction::VersionedTransaction;

const COMPUTE_BUDGET_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("ComputeBudget111111111111111111111111111111");
const SYSTEM_PROGRAM_ID: Pubkey = solana_sdk::pubkey!("11111111111111111111111111111111");
#[cfg(test)]
const PUMPFUN_INVOKE_LOG: &str = "Program 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P invoke";
#[cfg(test)]
const PUMPFUN_SUCCESS_LOG: &str = "Program 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P success";
const JITO_TIP_ACCOUNTS: [Pubkey; 8] = [
    solana_sdk::pubkey!("HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe"),
    solana_sdk::pubkey!("3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT"),
    solana_sdk::pubkey!("ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt"),
    solana_sdk::pubkey!("DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh"),
    solana_sdk::pubkey!("DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL"),
    solana_sdk::pubkey!("96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5"),
    solana_sdk::pubkey!("Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY"),
    solana_sdk::pubkey!("ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49"),
];
const SOLT0_TIP_ACCOUNTS: [Pubkey; 10] = [
    solana_sdk::pubkey!("DiTmWENJsHQdawVUUKnUXkconcpW4Jv52TnMWhkncF6t"),
    solana_sdk::pubkey!("HRyRhQ86t3H4aAtgvHVpUJmw64BDrb61gRiKcdKUXs5c"),
    solana_sdk::pubkey!("7y4whZmw388w1ggjToDLSBLv47drw5SUXcLk6jtmwixd"),
    solana_sdk::pubkey!("J9BMEWFbCBEjtQ1fG5Lo9kouX1HfrKQxeUxetwXrifBw"),
    solana_sdk::pubkey!("8U1JPQh3mVQ4F5jwRdFTBzvNRQaYFQppHQYoH38DJGSQ"),
    solana_sdk::pubkey!("Eb2KpSC8uMt9GmzyAEm5Eb1AAAgTjRaXWFjKyFXHZxF3"),
    solana_sdk::pubkey!("FCjUJZ1qozm1e8romw216qyfQMaaWKxWsuySnumVCCNe"),
    solana_sdk::pubkey!("ENxTEjSQ1YabmUpXAdCgevnHQ9MHdLv8tzFiuiYJqa13"),
    solana_sdk::pubkey!("6rYLG55Q9RpsPGvqdPNJs4z5WTxJVatMB8zV3WJhs5EK"),
    solana_sdk::pubkey!("Cix2bHfqPcKcM233mzxbLk14kSggUUiz2A87fJtGivXr"),
];

impl LogFilter {
    pub const CREATE_TOKEN_IX: &[u8] = &[24, 30, 200, 40, 5, 28, 7, 119];
    pub const BUY_IX: &[u8] = &[102, 6, 61, 18, 1, 218, 235, 234];
    pub const SELL_IX: &[u8] = &[51, 230, 133, 164, 1, 127, 131, 173];
    pub const BUY_EXACT_IN: &[u8] = &[250, 234, 13, 123, 213, 156, 19, 236];
    pub const BUY_EXACT_OUT: &[u8] = &[24, 211, 116, 40, 105, 3, 153, 56];
    pub const SELL_EXACT_IN: &[u8] = &[149, 39, 222, 155, 211, 124, 152, 26];
    pub const SELL_EXACT_OUT: &[u8] = &[95, 200, 71, 34, 8, 9, 11, 166];
    pub const INITIALIZE: &[u8] = &[175, 175, 109, 31, 13, 152, 155, 237];
    pub const INITIALIZE_V2: &[u8] = &[67, 153, 175, 39, 218, 16, 38, 32];

    /// Parse transaction logs and return instruction type and data
    pub fn parse_compiled_instruction(
        versioned_tx: &VersionedTransaction,
        bot_wallet: Option<Pubkey>,
    ) -> ClientResult<Vec<DexInstruction>> {
        let compiled_instructions = versioned_tx.message.instructions();
        let accounts = versioned_tx.message.static_account_keys();
        let pump_index = accounts.iter().position(|key| key == &PUMPFUN_PROGRAM_ID);
        let mut instructions: Vec<DexInstruction> = Vec::new();
        for (index, ix) in compiled_instructions.iter().enumerate() {
            if let Some(raw) = Self::raw_instruction(
                accounts,
                usize::from(ix.program_id_index),
                &ix.accounts,
                &ix.data,
                index,
                versioned_tx.signatures.first().copied().unwrap_or_default(),
                0,
            ) {
                instructions.push(DexInstruction::RawInstruction(Box::new(raw)));
            }
        }
        if let Some(index) = pump_index {
            for instruction in compiled_instructions {
                if instruction.program_id_index as usize == index {
                    let all_accounts_valid = instruction
                        .accounts
                        .iter()
                        .all(|&acc_idx| (acc_idx as usize) < accounts.len());
                    if !all_accounts_valid {
                        continue;
                    }
                    match instruction.data.get(0..8) {
                        // create
                        Some(Self::CREATE_TOKEN_IX) => {
                            if let Ok(token_info) =
                                parse_instruction_create_token_data(instruction, accounts)
                            {
                                instructions.push(DexInstruction::CreateToken(token_info));
                            };
                        }
                        // buy
                        Some(Self::BUY_IX)
                            if instruction.data.len() == 24 && instruction.accounts.len() >= 12 =>
                        {
                            if let Ok(trade_info) =
                                parse_instruction_trade_data(instruction, accounts, true)
                            {
                                if let Some(bot_wallet_pubkey) = bot_wallet {
                                    if trade_info.user == bot_wallet_pubkey {
                                        instructions.push(DexInstruction::BotTrade(trade_info));
                                    } else {
                                        instructions.push(DexInstruction::UserTrade(trade_info));
                                    }
                                } else {
                                    instructions.push(DexInstruction::UserTrade(trade_info));
                                }
                            };
                        }
                        Some(Self::SELL_IX)
                            if instruction.data.len() == 24 && instruction.accounts.len() >= 12 =>
                        {
                            if let Ok(trade_info) =
                                parse_instruction_trade_data(instruction, accounts, false)
                            {
                                if bot_wallet == Some(trade_info.user) {
                                    instructions.push(DexInstruction::BotTrade(trade_info));
                                } else {
                                    instructions.push(DexInstruction::UserTrade(trade_info));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let pump_index_2 = accounts
            .iter()
            .position(|key| key == &RAYDIUM_LAUNCHLAB_PROGRAM_ID);
        if let Some(index) = pump_index_2 {
            for instruction in compiled_instructions {
                if instruction.program_id_index as usize == index {
                    let all_accounts_valid = instruction
                        .accounts
                        .iter()
                        .all(|&acc_idx| (acc_idx as usize) < accounts.len());
                    if !all_accounts_valid {
                        continue;
                    }

                    // 检查指令数据长度
                    if instruction.data.len() < 8 {
                        continue;
                    }

                    match instruction.data.get(0..8) {
                        Some(Self::INITIALIZE) | Some(Self::INITIALIZE_V2) => {
                            match parse_instruction_bonk_create_token_data(instruction, accounts) {
                                Ok(token_info) => {
                                    instructions.push(DexInstruction::BonkCreateToken(token_info));
                                }
                                Err(e) => {
                                    log::debug!("failed to parse Raydium LaunchLab create: {e}");
                                }
                            }
                        }
                        Some(Self::BUY_EXACT_IN) => {
                            if let Ok(trade_request) =
                                parse_bonk_trade_data(instruction, accounts, TradeType::BuyExactIn)
                            {
                                instructions.push(DexInstruction::BonkTrade(trade_request));
                            }
                        }
                        Some(Self::SELL_EXACT_IN) => {
                            if let Ok(trade_request) =
                                parse_bonk_trade_data(instruction, accounts, TradeType::SellExactIn)
                            {
                                instructions.push(DexInstruction::BonkTrade(trade_request));
                            }
                        }
                        Some(Self::SELL_EXACT_OUT) => {
                            if let Ok(trade_request) = parse_bonk_trade_data(
                                instruction,
                                accounts,
                                TradeType::SellExactOut,
                            ) {
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

    fn raw_instruction(
        keys: &[Pubkey],
        program_index: usize,
        accounts: &[u8],
        data: &[u8],
        instruction_index: usize,
        signature: solana_sdk::signature::Signature,
        slot: u64,
    ) -> Option<crate::core::events::RawInstructionEvent> {
        let program_id = *keys.get(program_index)?;
        if !crate::shredstream::pump_ix::is_unknown_program_for_raw(&program_id) {
            return None;
        }
        let dexes = crate::core::events::raw_instruction_dexes(
            program_id,
            &accounts
                .iter()
                .map(|i| keys.get(*i as usize).copied())
                .collect::<Vec<_>>(),
        );
        // This legacy API is specifically a PumpFun subscription.
        if !dexes
            .iter()
            .any(|dex| dex.dex_type == crate::core::events::RawInstructionDexType::PumpFun)
        {
            return None;
        }
        Some(crate::core::events::RawInstructionEvent {
            dexes,
            metadata: crate::core::EventMetadata {
                signature,
                slot,
                ..Default::default()
            },
            program_id,
            instruction_index,
            accounts: accounts
                .iter()
                .map(|i| keys.get(usize::from(*i)).copied())
                .collect(),
            account_indices: accounts.to_vec(),
            instruction_data: data.to_vec(),
        })
    }

    fn convert_proto_instruction(
        proto_ix: &crate::shredstream::CompiledInstruction,
    ) -> ClientResult<SolanaCompiledInstruction> {
        Ok(SolanaCompiledInstruction {
            program_id_index: u8::try_from(proto_ix.program_id_index).map_err(|_| {
                ClientError::InvalidData(format!(
                    "program id index {} exceeds u8 range",
                    proto_ix.program_id_index
                ))
            })?,
            accounts: proto_ix.accounts.clone(),
            data: proto_ix.data.clone(),
        })
    }

    fn parse_proto_pubkeys(accounts: &[Vec<u8>]) -> ClientResult<Vec<Pubkey>> {
        accounts
            .iter()
            .enumerate()
            .map(|(index, key_bytes)| {
                let bytes: [u8; 32] = key_bytes.as_slice().try_into().map_err(|_| {
                    ClientError::InvalidData(format!(
                        "account key {index} has length {}, expected 32",
                        key_bytes.len()
                    ))
                })?;
                Ok(Pubkey::new_from_array(bytes))
            })
            .collect()
    }

    pub fn parse_compiled_instruction_shreder(
        tx_info: &SubscribeTransactionsResponse,
    ) -> ClientResult<Vec<DexInstruction>> {
        let transaction_update = tx_info
            .transaction
            .as_ref()
            .ok_or_else(|| ClientError::InvalidData("missing transaction update".to_string()))?;
        let transaction = transaction_update
            .transaction
            .as_ref()
            .ok_or_else(|| ClientError::InvalidData("missing transaction".to_string()))?;
        let message = transaction
            .message
            .as_ref()
            .ok_or_else(|| ClientError::InvalidData("missing transaction message".to_string()))?;
        let compiled_instructions = &message.instructions;
        let accounts = &message.account_keys;
        let accounts_pubkeys = Self::parse_proto_pubkeys(accounts)?;
        let mut instructions: Vec<DexInstruction> = Vec::new();
        for (index, ix) in compiled_instructions.iter().enumerate() {
            let signature = transaction
                .signatures
                .first()
                .and_then(|bytes| solana_sdk::signature::Signature::try_from(bytes.as_slice()).ok())
                .unwrap_or_default();
            if let Some(raw) = Self::raw_instruction(
                &accounts_pubkeys,
                ix.program_id_index as usize,
                &ix.accounts,
                &ix.data,
                index,
                signature,
                transaction_update.slot,
            ) {
                instructions.push(DexInstruction::RawInstruction(Box::new(raw)));
            }
        }
        if let Some(pump_index) = accounts_pubkeys
            .iter()
            .position(|key| key == &PUMPFUN_PROGRAM_ID)
        {
            for instruction in compiled_instructions {
                if usize::try_from(instruction.program_id_index).ok() != Some(pump_index)
                    || !instruction
                        .accounts
                        .iter()
                        .all(|&account_index| usize::from(account_index) < accounts.len())
                {
                    continue;
                }

                let solana_ix = Self::convert_proto_instruction(instruction)?;
                match instruction.data.get(0..8) {
                    Some(Self::CREATE_TOKEN_IX) => {
                        if let Ok(token_info) =
                            parse_instruction_create_token_data(&solana_ix, &accounts_pubkeys)
                        {
                            instructions.push(DexInstruction::CreateToken(token_info));
                        }
                    }
                    Some(Self::BUY_IX) if instruction.accounts.len() > 7 => {
                        if let Ok(trade_info) =
                            parse_instruction_trade_data(&solana_ix, &accounts_pubkeys, true)
                        {
                            instructions.push(DexInstruction::UserTrade(trade_info));
                        }
                    }
                    Some(Self::SELL_IX)
                        if instruction.data.len() == 24 && instruction.accounts.len() >= 12 =>
                    {
                        if let Ok(trade_info) =
                            parse_instruction_trade_data(&solana_ix, &accounts_pubkeys, false)
                        {
                            instructions.push(DexInstruction::UserTrade(trade_info));
                        }
                    }
                    _ => {}
                }
            }
        }

        let pump_index_2 = accounts_pubkeys
            .iter()
            .position(|key| key == &RAYDIUM_LAUNCHLAB_PROGRAM_ID);
        if let Some(index) = pump_index_2 {
            for instruction in compiled_instructions {
                if instruction.program_id_index as usize == index {
                    let all_accounts_valid = instruction
                        .accounts
                        .iter()
                        .all(|&acc_idx| (acc_idx as usize) < accounts.len());
                    if !all_accounts_valid {
                        continue;
                    }

                    // 检查指令数据长度
                    if instruction.data.len() < 8 {
                        continue;
                    }
                    let solana_ix = Self::convert_proto_instruction(instruction)?;

                    match instruction.data.get(0..8) {
                        Some(Self::INITIALIZE) | Some(Self::INITIALIZE_V2) => {
                            match parse_instruction_bonk_create_token_data(
                                &solana_ix,
                                &accounts_pubkeys,
                            ) {
                                Ok(token_info) => {
                                    instructions.push(DexInstruction::BonkCreateToken(token_info));
                                }
                                Err(e) => {
                                    log::debug!("failed to parse Raydium LaunchLab create: {e}");
                                }
                            }
                        }
                        Some(Self::BUY_EXACT_IN) => {
                            if let Ok(trade_request) = parse_bonk_trade_data(
                                &solana_ix,
                                &accounts_pubkeys,
                                TradeType::BuyExactIn,
                            ) {
                                instructions.push(DexInstruction::BonkTrade(trade_request));
                            }
                        }
                        Some(Self::SELL_EXACT_IN) => {
                            if let Ok(trade_request) = parse_bonk_trade_data(
                                &solana_ix,
                                &accounts_pubkeys,
                                TradeType::SellExactIn,
                            ) {
                                instructions.push(DexInstruction::BonkTrade(trade_request));
                            }
                        }
                        Some(Self::SELL_EXACT_OUT) => {
                            if let Ok(trade_request) = parse_bonk_trade_data(
                                &solana_ix,
                                &accounts_pubkeys,
                                TradeType::SellExactOut,
                            ) {
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

    pub fn parse_tip_info_shreder(
        tx_info: &SubscribeTransactionsResponse,
    ) -> (Option<u32>, Option<u64>, Option<String>, Option<u64>) {
        let Some(message) = tx_info
            .transaction
            .as_ref()
            .and_then(|update| update.transaction.as_ref())
            .and_then(|transaction| transaction.message.as_ref())
        else {
            return (None, None, None, None);
        };
        let compiled_instructions = &message.instructions;
        let accounts = &message.account_keys;
        let Ok(accounts_pubkeys) = Self::parse_proto_pubkeys(accounts) else {
            return (None, None, None, None);
        };

        let mut unit_limit: Option<u32> = None;
        let mut unit_price: Option<u64> = None;
        let mut fee_merchant: Option<String> = None;
        let mut fee: Option<u64> = None;

        for proto_ix in compiled_instructions {
            let Ok(instruction) = Self::convert_proto_instruction(proto_ix) else {
                continue;
            };
            let Some(program_id) = accounts_pubkeys.get(instruction.program_id_index as usize)
            else {
                continue;
            };
            if program_id == &COMPUTE_BUDGET_PROGRAM_ID && !instruction.data.is_empty() {
                match instruction.data[0] {
                    2 => {
                        if instruction.data.len() >= 5 {
                            unit_limit = Some(u32::from_le_bytes([
                                instruction.data[1],
                                instruction.data[2],
                                instruction.data[3],
                                instruction.data[4],
                            ]));
                        }
                    }
                    3 => {
                        if instruction.data.len() >= 9 {
                            unit_price = Some(u64::from_le_bytes([
                                instruction.data[1],
                                instruction.data[2],
                                instruction.data[3],
                                instruction.data[4],
                                instruction.data[5],
                                instruction.data[6],
                                instruction.data[7],
                                instruction.data[8],
                            ]));
                        }
                    }
                    _ => {}
                }
            }
            // 识别 System Program 转账
            if program_id == &SYSTEM_PROGRAM_ID
                && instruction.data.len() >= 12
                && instruction.data[0] == 2
                && instruction.data[1] == 0
                && instruction.data[2] == 0
                && instruction.data[3] == 0
                && instruction.accounts.len() >= 2
            {
                // 解析收款人
                let to_index = instruction.accounts[1] as usize;
                if to_index < accounts_pubkeys.len() {
                    let recipient = &accounts_pubkeys[to_index];
                    let amount = u64::from_le_bytes([
                        instruction.data[4],
                        instruction.data[5],
                        instruction.data[6],
                        instruction.data[7],
                        instruction.data[8],
                        instruction.data[9],
                        instruction.data[10],
                        instruction.data[11],
                    ]);
                    let tip_type = Self::get_tip_type(recipient);
                    fee_merchant = Some(tip_type);
                    fee = Some(amount);
                }
            }
        }

        (unit_limit, unit_price, fee_merchant, fee)
    }

    pub fn parse_tip_info(
        versioned_tx: &VersionedTransaction,
    ) -> (Option<u32>, Option<u64>, Option<String>, Option<u64>) {
        let compiled_instructions = versioned_tx.message.instructions();
        let accounts = versioned_tx.message.static_account_keys();
        let mut unit_limit: Option<u32> = None;
        let mut unit_price: Option<u64> = None;
        let mut fee_merchant: Option<String> = None;
        let mut fee: Option<u64> = None;

        for instruction in compiled_instructions {
            let Some(program_id) = accounts.get(instruction.program_id_index as usize) else {
                continue;
            };

            if program_id == &COMPUTE_BUDGET_PROGRAM_ID && !instruction.data.is_empty() {
                match instruction.data[0] {
                    2 => {
                        if instruction.data.len() >= 5 {
                            unit_limit = Some(u32::from_le_bytes([
                                instruction.data[1],
                                instruction.data[2],
                                instruction.data[3],
                                instruction.data[4],
                            ]));
                        }
                    }
                    3 => {
                        if instruction.data.len() >= 9 {
                            unit_price = Some(u64::from_le_bytes([
                                instruction.data[1],
                                instruction.data[2],
                                instruction.data[3],
                                instruction.data[4],
                                instruction.data[5],
                                instruction.data[6],
                                instruction.data[7],
                                instruction.data[8],
                            ]));
                        }
                    }
                    _ => {}
                }
            }
            // 识别 System Program 转账
            if program_id == &SYSTEM_PROGRAM_ID
                && instruction.data.len() >= 12
                && instruction.data[0] == 2
                && instruction.data[1] == 0
                && instruction.data[2] == 0
                && instruction.data[3] == 0
                && instruction.accounts.len() >= 2
            {
                // 解析收款人
                let to_index = instruction.accounts[1] as usize;
                if to_index < accounts.len() {
                    let recipient = &accounts[to_index];
                    let amount = u64::from_le_bytes([
                        instruction.data[4],
                        instruction.data[5],
                        instruction.data[6],
                        instruction.data[7],
                        instruction.data[8],
                        instruction.data[9],
                        instruction.data[10],
                        instruction.data[11],
                    ]);
                    let tip_type = Self::get_tip_type(recipient);
                    fee_merchant = Some(tip_type);
                    fee = Some(amount);
                }
            }
        }

        (unit_limit, unit_price, fee_merchant, fee)
    }

    fn get_tip_type(recipient: &Pubkey) -> String {
        if JITO_TIP_ACCOUNTS.contains(recipient) {
            "JITO".to_string()
        } else if SOLT0_TIP_ACCOUNTS.contains(recipient) {
            "SOLT0".to_string()
        } else {
            let recipient = recipient.to_string();
            if recipient.starts_with("node") {
                "NODE".to_string()
            } else if recipient.starts_with("noz") || recipient.starts_with("TEMP") {
                "TEMP".to_string()
            } else if recipient.starts_with("Next")
                || recipient.starts_with("neXt")
                || recipient.starts_with("next")
            {
                "NEXT".to_string()
            } else {
                "UNKNOWN".to_string()
            }
        }
    }

    /// Parse transaction logs and return instruction type and data
    pub fn parse_instruction(
        logs: &[String],
        bot_wallet: Option<Pubkey>,
    ) -> ClientResult<Vec<DexInstruction>> {
        // Each invocation owns its events until it succeeds. Successful CPI legs
        // are committed to their parent, so a failing router rolls them all back.
        struct Frame {
            program: String,
            events: Vec<DexInstruction>,
        }
        let mut stack: Vec<Frame> = Vec::new();
        let mut instructions = Vec::new();
        for log in logs {
            if let Some(rest) = log.strip_prefix("Program ") {
                if let Some((program, depth)) = rest.split_once(" invoke [") {
                    let Some(depth) = depth
                        .strip_suffix(']')
                        .and_then(|d| d.parse::<usize>().ok())
                    else {
                        continue;
                    };
                    if depth == 0 {
                        continue;
                    }
                    stack.truncate(depth - 1);
                    if stack.len() != depth - 1 {
                        continue;
                    }
                    stack.push(Frame {
                        program: program.to_owned(),
                        events: Vec::new(),
                    });
                    continue;
                }
                let completion = rest
                    .strip_suffix(" success")
                    .map(|p| (p, true))
                    .or_else(|| rest.split_once(" failed:").map(|(p, _)| (p, false)));
                if let Some((program, success)) = completion {
                    if stack.last().is_none_or(|f| f.program != program) {
                        continue;
                    }
                    let frame = stack.pop().unwrap();
                    if success {
                        if let Some(parent) = stack.last_mut() {
                            parent.events.extend(frame.events);
                        } else {
                            instructions.extend(frame.events);
                        }
                    } else if stack.is_empty() {
                        // A failed top-level instruction rolls back the transaction.
                        instructions.clear();
                        break;
                    }
                    continue;
                }
            }
            let Some(frame) = stack.last_mut() else {
                continue;
            };
            if frame.program != PUMPFUN_PROGRAM_ID.to_string() {
                continue;
            }
            let Some(encoded) = log.strip_prefix("Program data: ") else {
                continue;
            };
            use base64::Engine as _;
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
                continue;
            };
            match bytes.get(..8) {
                Some([189, 219, 127, 211, 78, 230, 97, 238]) => {
                    if let Ok(trade) = parse_trade_data(encoded) {
                        frame.events.push(if bot_wallet == Some(trade.user) {
                            DexInstruction::BotTrade(trade)
                        } else {
                            DexInstruction::UserTrade(trade)
                        });
                    }
                }
                Some([27, 114, 169, 77, 222, 235, 99, 118]) => {
                    if let Ok(token) = parse_create_token_data(encoded) {
                        frame.events.push(DexInstruction::CreateToken(token));
                    }
                }
                _ => {}
            }
        }

        Ok(instructions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shredstream::{
        CompiledInstruction, Message, SubscribeUpdateTransaction, Transaction,
    };
    use base64::Engine as _;
    use solana_sdk::{
        hash::Hash,
        message::{v0, MessageHeader, VersionedMessage},
        signature::Signature,
    };

    fn response(
        account_keys: Vec<Vec<u8>>,
        instructions: Vec<CompiledInstruction>,
    ) -> SubscribeTransactionsResponse {
        SubscribeTransactionsResponse {
            transaction: Some(SubscribeUpdateTransaction {
                transaction: Some(Transaction {
                    signatures: vec![vec![7; 64]],
                    message: Some(Message {
                        account_keys,
                        instructions,
                        ..Message::default()
                    }),
                }),
                slot: 42,
            }),
            ..SubscribeTransactionsResponse::default()
        }
    }

    fn trade_log_data(user: Pubkey, trailing_bytes: usize) -> String {
        let mut data = vec![0u8; 129 + trailing_bytes];
        data[..8].copy_from_slice(&[189, 219, 127, 211, 78, 230, 97, 238]);
        data[40..48].copy_from_slice(&11u64.to_le_bytes());
        data[48..56].copy_from_slice(&22u64.to_le_bytes());
        data[56] = 1;
        data[57..89].copy_from_slice(user.as_ref());
        base64::engine::general_purpose::STANDARD.encode(data)
    }

    #[test]
    fn protobuf_parser_requires_transaction_fields() {
        let response = SubscribeTransactionsResponse::default();
        assert!(LogFilter::parse_compiled_instruction_shreder(&response).is_err());
        assert_eq!(
            LogFilter::parse_tip_info_shreder(&response),
            (None, None, None, None)
        );
    }

    #[test]
    fn protobuf_parser_rejects_malformed_pubkeys() {
        let response = response(vec![vec![0; 31]], vec![]);
        assert!(LogFilter::parse_compiled_instruction_shreder(&response).is_err());
    }

    #[test]
    fn protobuf_instruction_rejects_truncated_program_index() {
        let instruction = CompiledInstruction {
            program_id_index: 256,
            accounts: vec![],
            data: vec![],
        };
        assert!(LogFilter::convert_proto_instruction(&instruction).is_err());
    }

    #[test]
    fn protobuf_parser_does_not_match_discriminator_from_other_program() {
        let mut data = LogFilter::BUY_IX.to_vec();
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(&2u64.to_le_bytes());
        let instruction = CompiledInstruction {
            program_id_index: 0,
            accounts: vec![0; 8],
            data,
        };
        let response = response(
            vec![Pubkey::new_unique().to_bytes().to_vec()],
            vec![instruction],
        );

        let parsed = LogFilter::parse_compiled_instruction_shreder(&response)
            .expect("well-formed protobuf transaction");
        assert!(
            parsed.is_empty(),
            "unrelated programs must not reach PumpFun callbacks"
        );
    }

    #[test]
    fn protobuf_tip_parser_ignores_invalid_program_index() {
        let instruction = CompiledInstruction {
            program_id_index: 999,
            accounts: vec![],
            data: vec![2, 1, 0, 0, 0],
        };
        let response = response(
            vec![COMPUTE_BUDGET_PROGRAM_ID.to_bytes().to_vec()],
            vec![instruction],
        );
        assert_eq!(
            LogFilter::parse_tip_info_shreder(&response),
            (None, None, None, None)
        );
    }

    #[test]
    fn versioned_transaction_parser_emits_sell_trades() {
        let mut data = LogFilter::SELL_IX.to_vec();
        data.extend_from_slice(&123u64.to_le_bytes());
        data.extend_from_slice(&456u64.to_le_bytes());
        let mut account_keys = vec![PUMPFUN_PROGRAM_ID];
        account_keys.extend((0..12).map(|_| Pubkey::new_unique()));
        let transaction = VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::V0(v0::Message {
                header: MessageHeader {
                    num_required_signatures: 1,
                    num_readonly_signed_accounts: 0,
                    num_readonly_unsigned_accounts: 0,
                },
                account_keys,
                recent_blockhash: Hash::default(),
                instructions: vec![SolanaCompiledInstruction::new_from_raw_parts(
                    0,
                    data,
                    (1..=12).collect(),
                )],
                address_table_lookups: vec![],
            }),
        };

        let parsed = LogFilter::parse_compiled_instruction(&transaction, None)
            .expect("well-formed transaction");
        assert!(matches!(
            parsed.as_slice(),
            [DexInstruction::UserTrade(trade)]
                if !trade.is_buy && trade.token_amount == 123 && trade.sol_amount == 456
        ));
    }

    #[test]
    fn log_parser_ignores_nested_program_data() {
        let nested_user = Pubkey::new_unique();
        let pump_user = Pubkey::new_unique();
        let logs = vec![
            format!("{PUMPFUN_INVOKE_LOG} [1]"),
            "Program log: Instruction: Buy".to_string(),
            "Program 11111111111111111111111111111111 invoke [2]".to_string(),
            format!("Program data: {}", trade_log_data(nested_user, 64)),
            "Program 11111111111111111111111111111111 success".to_string(),
            format!("Program data: {}", trade_log_data(pump_user, 0)),
            PUMPFUN_SUCCESS_LOG.to_string(),
        ];

        let parsed = LogFilter::parse_instruction(&logs, None).expect("well-formed logs");
        assert!(
            matches!(
                parsed.as_slice(),
                [DexInstruction::UserTrade(trade)] if trade.user == pump_user
            ),
            "parsed events: {parsed:?}"
        );
    }

    #[test]
    fn failed_pump_invocation_does_not_emit_events() {
        let logs = vec![
            format!("{PUMPFUN_INVOKE_LOG} [1]"),
            "Program log: Instruction: Buy".to_string(),
            format!("Program data: {}", trade_log_data(Pubkey::new_unique(), 0)),
            "Program 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P failed: custom program error: 0x1"
                .to_string(),
        ];

        let parsed = LogFilter::parse_instruction(&logs, None).expect("well-formed logs");
        assert!(parsed.is_empty());
    }
}
