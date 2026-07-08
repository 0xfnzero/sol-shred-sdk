//! Compatibility re-exports for parser modules migrated from sol-parser-sdk.

use std::collections::HashMap;

use crate::grpc::types::Protocol;

pub const PUMPFUN_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
pub const PUMPSWAP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
pub const PUMPSWAP_FEES_PROGRAM_ID: &str = "pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ";
pub const RAYDIUM_LAUNCHLAB_PROGRAM_ID: &str = "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj";
pub const RAYDIUM_CPMM_PROGRAM_ID: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
pub const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
pub const RAYDIUM_AMM_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
pub const ORCA_WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
pub const METEORA_POOLS_PROGRAM_ID: &str = "Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB";
pub const METEORA_DAMM_V2_PROGRAM_ID: &str = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";
pub const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
pub const METEORA_DBC_PROGRAM_ID: &str = "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN";

pub use crate::instr::program_ids::{
    METEORA_DAMM_V2_PROGRAM_ID as METEORA_DAMM_V2_PROGRAM,
    METEORA_DBC_PROGRAM_ID as METEORA_DBC_PROGRAM, METEORA_DLMM_PROGRAM_ID as METEORA_DLMM_PROGRAM,
    METEORA_POOLS_PROGRAM_ID as METEORA_POOLS_PROGRAM,
    ORCA_WHIRLPOOL_PROGRAM_ID as ORCA_WHIRLPOOL_PROGRAM, PUMPFUN_PROGRAM_ID as PUMPFUN_PROGRAM,
    PUMPSWAP_PROGRAM_ID as PUMPSWAP_PROGRAM, PUMP_FEES_PROGRAM_ID as PUMP_FEES_PROGRAM,
    RAYDIUM_AMM_V4_PROGRAM_ID as RAYDIUM_AMM_V4_PROGRAM,
    RAYDIUM_CLMM_PROGRAM_ID as RAYDIUM_CLMM_PROGRAM,
    RAYDIUM_CPMM_PROGRAM_ID as RAYDIUM_CPMM_PROGRAM,
    RAYDIUM_LAUNCHLAB_PROGRAM_ID as RAYDIUM_LAUNCHLAB_PROGRAM,
};

pub use crate::instr::program_ids::PUMP_FEES_PROGRAM_ID as PUMPSWAP_FEES_PROGRAM;

lazy_static::lazy_static! {
    pub static ref PROTOCOL_PROGRAM_IDS: HashMap<Protocol, Vec<&'static str>> = {
        let mut map = HashMap::new();
        map.insert(Protocol::PumpFun, vec![PUMPFUN_PROGRAM_ID]);
        map.insert(Protocol::PumpSwap, vec![PUMPSWAP_PROGRAM_ID]);
        map.insert(Protocol::PumpFees, vec![PUMPSWAP_FEES_PROGRAM_ID]);
        map.insert(Protocol::RaydiumLaunchlab, vec![RAYDIUM_LAUNCHLAB_PROGRAM_ID]);
        map.insert(Protocol::RaydiumCpmm, vec![RAYDIUM_CPMM_PROGRAM_ID]);
        map.insert(Protocol::RaydiumClmm, vec![RAYDIUM_CLMM_PROGRAM_ID]);
        map.insert(Protocol::RaydiumAmmV4, vec![RAYDIUM_AMM_V4_PROGRAM_ID]);
        map.insert(Protocol::OrcaWhirlpool, vec![ORCA_WHIRLPOOL_PROGRAM_ID]);
        map.insert(Protocol::MeteoraPools, vec![METEORA_POOLS_PROGRAM_ID]);
        map.insert(Protocol::MeteoraDammV2, vec![METEORA_DAMM_V2_PROGRAM_ID]);
        map.insert(Protocol::MeteoraDlmm, vec![METEORA_DLMM_PROGRAM_ID]);
        map.insert(Protocol::MeteoraDbc, vec![METEORA_DBC_PROGRAM_ID]);
        map
    };
}

pub fn get_program_ids_for_protocols(protocols: &[Protocol]) -> Vec<String> {
    let mut program_ids = Vec::new();
    for protocol in protocols {
        if let Some(ids) = PROTOCOL_PROGRAM_IDS.get(protocol) {
            for id in ids {
                program_ids.push(id.to_string());
            }
        }
    }
    program_ids.sort();
    program_ids.dedup();
    program_ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instr;

    #[test]
    fn grpc_program_ids_match_instruction_program_ids() {
        assert_eq!(PUMPFUN_PROGRAM, instr::program_ids::PUMPFUN_PROGRAM_ID);
        assert_eq!(PUMPSWAP_PROGRAM, instr::program_ids::PUMPSWAP_PROGRAM_ID);
        assert_eq!(
            PUMPSWAP_FEES_PROGRAM,
            instr::program_ids::PUMP_FEES_PROGRAM_ID
        );
        assert_eq!(
            RAYDIUM_LAUNCHLAB_PROGRAM,
            instr::program_ids::RAYDIUM_LAUNCHLAB_PROGRAM_ID
        );
        assert_eq!(
            RAYDIUM_CPMM_PROGRAM,
            instr::program_ids::RAYDIUM_CPMM_PROGRAM_ID
        );
        assert_eq!(
            RAYDIUM_CLMM_PROGRAM,
            instr::program_ids::RAYDIUM_CLMM_PROGRAM_ID
        );
        assert_eq!(
            RAYDIUM_AMM_V4_PROGRAM,
            instr::program_ids::RAYDIUM_AMM_V4_PROGRAM_ID
        );
        assert_eq!(
            ORCA_WHIRLPOOL_PROGRAM,
            instr::program_ids::ORCA_WHIRLPOOL_PROGRAM_ID
        );
        assert_eq!(
            METEORA_POOLS_PROGRAM,
            instr::program_ids::METEORA_POOLS_PROGRAM_ID
        );
        assert_eq!(
            METEORA_DAMM_V2_PROGRAM,
            instr::program_ids::METEORA_DAMM_V2_PROGRAM_ID
        );
        assert_eq!(
            METEORA_DLMM_PROGRAM,
            instr::program_ids::METEORA_DLMM_PROGRAM_ID
        );
        assert_eq!(
            METEORA_DBC_PROGRAM,
            instr::program_ids::METEORA_DBC_PROGRAM_ID
        );
    }

    #[test]
    fn protocol_filter_maps_all_supported_protocols() {
        let protocols = [
            Protocol::PumpFun,
            Protocol::PumpSwap,
            Protocol::PumpFees,
            Protocol::RaydiumLaunchlab,
            Protocol::RaydiumCpmm,
            Protocol::RaydiumClmm,
            Protocol::RaydiumAmmV4,
            Protocol::OrcaWhirlpool,
            Protocol::MeteoraPools,
            Protocol::MeteoraDammV2,
            Protocol::MeteoraDlmm,
            Protocol::MeteoraDbc,
        ];
        for protocol in protocols {
            assert!(
                PROTOCOL_PROGRAM_IDS.contains_key(&protocol),
                "missing program id mapping for {protocol:?}"
            );
        }
    }
}
