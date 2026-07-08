//! 账户相关的程序 ID 常量
//!
//! 定义用于账户解析的各种程序 ID

use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

// ==================== DEX 协议程序 ID ====================

/// PumpSwap 程序 ID
pub const PUMPSWAP_PROGRAM_ID: Pubkey = pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");

// ==================== 系统程序 ID ====================

/// SPL Token 程序 ID
pub const SPL_TOKEN_PROGRAM_ID: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/// SPL Token-2022 程序 ID
pub const SPL_TOKEN_2022_PROGRAM_ID: Pubkey =
    pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// System Program ID
pub const SYSTEM_PROGRAM_ID: Pubkey = pubkey!("11111111111111111111111111111111");
