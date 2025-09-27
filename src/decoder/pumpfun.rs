use std::io::Read;

use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
#[derive(Clone, Debug, PartialEq)]
pub enum PumpProgramIx {
    Create(CreateIxArgs),
    Buy(BuyIxArgs),
    Sell(SellIxArgs),
}

const PUMPFUN_PROGRAM_ID: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
// In your deserialization code
impl PumpProgramIx {
    pub fn deserialize_pumpfun(accounts: Vec<Pubkey>, buf: &[u8]) -> std::io::Result<Self> {
        // Check program ID first
        if !accounts.contains(&PUMPFUN_PROGRAM_ID) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Not a PumpFun program instruction",
            ));
        }

        let mut reader = buf;
        let mut maybe_discm = [0u8; 8];
        reader.read_exact(&mut maybe_discm)?;

        match maybe_discm {
            CREATE_IX_DISCM => Ok(Self::Create(CreateIxArgs::deserialize(&mut reader)?)),
            BUY_IX_DISCM => Ok(Self::Buy(BuyIxArgs::deserialize(&mut reader)?)),
            SELL_IX_DISCM => Ok(Self::Sell(SellIxArgs::deserialize(&mut reader)?)),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("PumpFun discm {:?} not found", maybe_discm),
            )),
        }
    }
}

pub const CREATE_IX_DISCM: [u8; 8] = [24, 30, 200, 40, 5, 28, 7, 119];
#[derive(
    BorshDeserialize, BorshSerialize, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize,
)]
pub struct CreateIxArgs {
    pub name: String,
    pub symbol: String,
    pub uri: String,
}

pub const SELL_IX_DISCM: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];
#[derive(
    BorshDeserialize, BorshSerialize, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize,
)]
pub struct SellIxArgs {
    pub amount: u64,
    pub min_sol_output: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SellIxData(pub SellIxArgs);
impl From<SellIxArgs> for SellIxData {
    fn from(args: SellIxArgs) -> Self {
        Self(args)
    }
}

pub const BUY_IX_DISCM: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
#[derive(
    BorshDeserialize, BorshSerialize, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize,
)]
pub struct BuyIxArgs {
    pub amount: u64,
    pub max_sol_cost: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct BuyIxData(pub BuyIxArgs);
impl From<BuyIxArgs> for BuyIxData {
    fn from(args: BuyIxArgs) -> Self {
        Self(args)
    }
}
