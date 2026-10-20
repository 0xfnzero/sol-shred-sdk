//! Trade provenance and evidence accessors for Shred instruction callbacks.
use super::{DexEvent, RawInstructionPayload, TradeParseStatus};

impl DexEvent {
    /// Original outer router instruction, when exact invocation provenance exists.
    pub fn third_program(&self) -> Option<&RawInstructionPayload> {
        match self {
            Self::PumpFunTrade(t)
            | Self::PumpFunBuy(t)
            | Self::PumpFunSell(t)
            | Self::PumpFunBuyExactSolIn(t) => t.third_program.as_ref(),
            Self::PumpSwapTrade(t) => t.third_program.as_ref(),
            Self::PumpSwapBuy(t) => t.third_program.as_ref(),
            Self::PumpSwapSell(t) => t.third_program.as_ref(),
            Self::RaydiumLaunchlabTrade(t) => t.third_program.as_ref(),
            Self::RaydiumCpmmSwap(t) => t.third_program.as_ref(),
            Self::RaydiumClmmSwap(t) => t.third_program.as_ref(),
            Self::RaydiumAmmV4Swap(t) => t.third_program.as_ref(),
            Self::OrcaWhirlpoolSwap(t) => t.third_program.as_ref(),
            Self::MeteoraPoolsSwap(t) => t.third_program.as_ref(),
            Self::MeteoraDammV2Swap(t) => t.third_program.as_ref(),
            Self::MeteoraDlmmSwap(t) => t.third_program.as_ref(),
            Self::MeteoraDbcSwap(t) => t.third_program.as_ref(),
            _ => None,
        }
    }

    /// Evidence level for a trade; non-trade events return None.
    pub fn trade_parse_status(&self) -> Option<TradeParseStatus> {
        match self {
            Self::PumpFunTrade(t)
            | Self::PumpFunBuy(t)
            | Self::PumpFunSell(t)
            | Self::PumpFunBuyExactSolIn(t) => Some(t.parse_status),
            Self::PumpSwapTrade(t) => Some(t.parse_status),
            Self::PumpSwapBuy(t) => Some(t.parse_status),
            Self::PumpSwapSell(t) => Some(t.parse_status),
            Self::RaydiumLaunchlabTrade(t) => Some(t.parse_status),
            Self::RaydiumCpmmSwap(t) => Some(t.parse_status),
            Self::RaydiumClmmSwap(t) => Some(t.parse_status),
            Self::RaydiumAmmV4Swap(t) => Some(t.parse_status),
            Self::OrcaWhirlpoolSwap(t) => Some(t.parse_status),
            Self::MeteoraPoolsSwap(t) => Some(t.parse_status),
            Self::MeteoraDammV2Swap(t) => Some(t.parse_status),
            Self::MeteoraDlmmSwap(t) => Some(t.parse_status),
            Self::MeteoraDbcSwap(t) => Some(t.parse_status),
            _ => None,
        }
    }
}
