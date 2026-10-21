//! Shared historical stock identities. Shred observations do not prove execution,
//! pool reserves, or prices; publish executable quotes only from account updates.
use crate::instr::program_ids::{
    METEORA_DLMM_PROGRAM_ID, ORCA_WHIRLPOOL_PROGRAM_ID, RAYDIUM_CLMM_PROGRAM_ID,
    RAYDIUM_CPMM_PROGRAM_ID,
};
use crate::transaction_route::{RouteSwapLeg, SwapProtocol, TransactionRoute};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::{collections::HashMap, str::FromStr};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StockToken {
    pub mint: String,
    pub symbol: String,
    pub decimals: u8,
    pub catalog_source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StockPool {
    pub pool: String,
    pub mint: String,
    pub quote_mint: String,
    pub dex_type: String,
    pub preferred: bool,
    pub liquid_side_reserve_usd: f64,
    pub nominal_snapshot_fee_bps: Option<f64>,
    pub catalog_fee_snapshot: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "CatalogData")]
pub struct StockTokenCatalog {
    pub tokens: Vec<StockToken>,
    pools: Vec<StockPool>,
    #[serde(skip)]
    index: HashMap<Pubkey, (Pubkey, Pubkey, Pubkey)>,
}
#[derive(Deserialize)]
struct CatalogData {
    tokens: Vec<StockToken>,
    pools: Vec<StockPool>,
}
impl TryFrom<CatalogData> for StockTokenCatalog {
    type Error = String;
    fn try_from(data: CatalogData) -> Result<Self, Self::Error> {
        Self::new(data.tokens, data.pools)
    }
}
impl StockTokenCatalog {
    /// Validate and compile immutable pool identities once, including after deserialization.
    pub fn new(tokens: Vec<StockToken>, pools: Vec<StockPool>) -> Result<Self, String> {
        let mut index = HashMap::new();
        for p in &pools {
            let program = match p.dex_type.as_str() {
                "raydium_clmm" => RAYDIUM_CLMM_PROGRAM_ID,
                "raydium_cpmm" => RAYDIUM_CPMM_PROGRAM_ID,
                "orca_whirlpool" => ORCA_WHIRLPOOL_PROGRAM_ID,
                "meteora_dlmm" => METEORA_DLMM_PROGRAM_ID,
                _ => return Err("Unsupported stock pool protocol".into()),
            };
            let parse = |s: &str| Pubkey::from_str(s).map_err(|e| e.to_string());
            let pool = parse(&p.pool)?;
            let a = parse(&p.mint)?;
            let b = parse(&p.quote_mint)?;
            if pool == Pubkey::default()
                || a == Pubkey::default()
                || b == Pubkey::default()
                || a == b
            {
                return Err("Invalid stock pool identity".into());
            }
            if index.insert(pool, (program, a, b)).is_some() {
                return Err("Duplicate stock pool identity".into());
            }
        }
        Ok(Self {
            tokens,
            pools,
            index,
        })
    }
    pub fn pools(&self) -> &[StockPool] {
        &self.pools
    }

    pub fn screened_defaults() -> Self {
        serde_json::from_str(include_str!("stock_token_catalog.json"))
            .expect("tested stock catalog")
    }
    pub fn preferred_pool(&self, mint: &str) -> Option<&StockPool> {
        self.pools.iter().find(|p| p.mint == mint)
    }
    /// Only verified legs of executed transactions. Unresolved Shred signals must
    /// remain unresolved until execution metadata and mint identities are available.
    pub fn conversion_legs<'a>(
        &'a self,
        route: &'a TransactionRoute,
    ) -> impl Iterator<Item = &'a RouteSwapLeg> + 'a {
        route.legs.iter().filter(move |leg| {
            if !route.succeeded {
                return false;
            }
            let program = match leg.protocol {
                SwapProtocol::RaydiumClmm => RAYDIUM_CLMM_PROGRAM_ID,
                SwapProtocol::RaydiumCpmm => RAYDIUM_CPMM_PROGRAM_ID,
                SwapProtocol::OrcaWhirlpool => ORCA_WHIRLPOOL_PROGRAM_ID,
                SwapProtocol::MeteoraDlmm => METEORA_DLMM_PROGRAM_ID,
                _ => return false,
            };
            let (Some(a), Some(b)) = (leg.input_mint, leg.output_mint) else {
                return false;
            };
            self.index
                .get(&leg.pool)
                .is_some_and(|&(expected, mint, quote)| {
                    program == expected
                        && leg.program == expected
                        && ((mint == a && quote == b) || (mint == b && quote == a))
                })
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_distinct_and_have_only_verified_bridge_quote_assets() {
        let c = StockTokenCatalog::screened_defaults();
        assert_eq!((c.tokens.len(), c.pools.len()), (34, 63));
        let mut seen = std::collections::HashSet::new();
        for p in c.pools {
            assert!(seen.insert(p.pool));
            assert!(p.liquid_side_reserve_usd >= 500.0);
            assert!(c.tokens.iter().any(|t| t.mint == p.mint));
            assert!([
                "So11111111111111111111111111111111111111112",
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
            ]
            .contains(&p.quote_mint.as_str()));
        }
    }
    #[test]
    fn indexed_matching_survives_deserialization_and_checks_identity() {
        use crate::transaction_route::InstructionPosition;
        let original = StockTokenCatalog::screened_defaults();
        let c: StockTokenCatalog =
            serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
        let (&pool, &(program, a, b)) = c.index.iter().next().unwrap();
        let protocol = if program == RAYDIUM_CLMM_PROGRAM_ID {
            SwapProtocol::RaydiumClmm
        } else if program == RAYDIUM_CPMM_PROGRAM_ID {
            SwapProtocol::RaydiumCpmm
        } else if program == ORCA_WHIRLPOOL_PROGRAM_ID {
            SwapProtocol::OrcaWhirlpool
        } else {
            SwapProtocol::MeteoraDlmm
        };
        let leg = RouteSwapLeg {
            position: InstructionPosition {
                outer_index: 0,
                inner_index: None,
                stack_height: None,
            },
            program,
            protocol,
            pool,
            trader: Pubkey::default(),
            input_account: Pubkey::default(),
            output_account: Pubkey::default(),
            input_mint: Some(a),
            output_mint: Some(b),
            amount_specified_is_input: true,
            specified_amount: 1,
            other_amount_threshold: 0,
            actual_input_amount: Some(1),
            actual_output_amount: Some(1),
            stonkfun_mode: None,
            stonkfun_graduated: false,
        };
        let mut route = TransactionRoute {
            signature: Default::default(),
            succeeded: true,
            legs: vec![leg],
            transfers: vec![],
            native_token_actions: vec![],
            unknown_invocations: vec![],
        };
        assert_eq!(c.conversion_legs(&route).count(), 1);
        route.legs[0].input_mint = Some(b);
        route.legs[0].output_mint = Some(a);
        assert_eq!(c.conversion_legs(&route).count(), 1);
        route.succeeded = false;
        assert_eq!(c.conversion_legs(&route).count(), 0);
        route.succeeded = true;
        route.legs[0].program = Pubkey::default();
        assert_eq!(c.conversion_legs(&route).count(), 0);
        route.legs[0].program = program;
        route.legs[0].input_mint = Some(Pubkey::new_unique());
        assert_eq!(c.conversion_legs(&route).count(), 0);
        let mut pools = c.pools().to_vec();
        pools.push(pools[0].clone());
        assert!(StockTokenCatalog::new(c.tokens.clone(), pools).is_err());
    }
}
