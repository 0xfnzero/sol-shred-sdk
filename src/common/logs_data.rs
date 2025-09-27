use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::{pubkey::Pubkey, transaction::VersionedTransaction};

use crate::common::error::{ClientError, ClientResult};

#[derive(Debug)]
pub enum DexInstruction {
    CreateToken(CreateTokenInfo),
    BonkCreateToken(BonkCreateTokenInfo),
    BonkUserTrade(TradeRequest), 
    BonkTrade(TradeRequest),
    UserTrade(TradeInfo),
    BotTrade(TradeInfo),
    Tip(TipInfo),
    Other,
}
#[derive(Debug, Clone, PartialEq, BorshDeserialize, BorshSerialize)]
#[borsh(use_discriminant = true)]
pub enum TradeType {
    BuyExactOut = 0,
    BuyExactIn = 1,
    SellExactIn = 2,
    SellExactOut = 3,
}

#[derive(Clone, Debug, Default, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CreateTokenInfo {
    pub slot: u64,
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub creator: Pubkey,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub user: Pubkey,
    pub unit_limit: u32,
    pub unit_price: u64,
    pub fee_merchant: String,
    pub fee: u64,
}

#[derive(Clone, Debug, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct TradeRequest {
    pub payer: String,
    pub base_mint: String,
    pub amount: u64,
    pub trade_type: TradeType,
}




#[derive(Clone, Debug, Default, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct BonkCreateTokenInfo {
    pub payer: String,
    pub creator: String,
    pub base_mint: String,
    pub pool_state: String,
    pub platform_config: String,
    pub virtual_base: f64,
    pub virtual_quote: f64,
    pub base_vault: String,
    pub quote_vault: String,
    pub symbol: String,
    pub name: String,
    pub uri: String,
    pub unit_limit: u32,
    pub unit_price: u64,
    pub fee_merchant: String, 
    pub fee: u64,
}

#[derive(Clone, Debug, Default, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct TradeInfo {
    pub slot: u64,
    pub mint: Pubkey,
    pub sol_amount: u64,
    pub token_amount: u64,
    pub is_buy: bool,
    pub user: Pubkey,
    pub timestamp: i64,
    pub virtual_sol_reserves: u64,
    pub virtual_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub real_token_reserves: u64,
}

#[derive(Clone, Debug, Default, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct TipInfo {
    pub slot: u64,
    pub signature: String,
}

#[derive(Clone, Debug, Default, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct CompleteInfo {
    pub user: Pubkey,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Default, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct SwapBaseInLog {
    pub log_type: u8,
    // input
    pub amount_in: u64,
    pub minimum_out: u64,
    pub direction: u64,
    // user info
    pub user_source: u64,
    // pool info
    pub pool_coin: u64,
    pub pool_pc: u64,
    // calc result
    pub out_amount: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransferInfo {
    pub slot: u64,
    pub signature: String,
    pub tx: Option<VersionedTransaction>,
}

pub trait EventTrait: Sized + std::fmt::Debug {
    fn from_bytes(bytes: &[u8]) -> ClientResult<Self>;
}

impl EventTrait for CreateTokenInfo {
    fn from_bytes(bytes: &[u8]) -> ClientResult<Self> {
        CreateTokenInfo::try_from_slice(bytes).map_err(|e| ClientError::Other(e.to_string()))
    }
}

impl EventTrait for TradeInfo {
    fn from_bytes(bytes: &[u8]) -> ClientResult<Self> {
        TradeInfo::try_from_slice(bytes).map_err(|e| ClientError::Other(e.to_string()))
    }
}

impl EventTrait for CompleteInfo {
    fn from_bytes(bytes: &[u8]) -> ClientResult<Self> {
        CompleteInfo::try_from_slice(bytes).map_err(|e| ClientError::Other(e.to_string()))
    }
}

impl EventTrait for SwapBaseInLog {
    fn from_bytes(bytes: &[u8]) -> ClientResult<Self> {
        SwapBaseInLog::try_from_slice(bytes).map_err(|e| ClientError::Other(e.to_string()))
    }
}