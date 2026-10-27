//! Trade provenance accessors for Shred instruction callbacks.
use super::{DexEvent, RawInstructionPayload};

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
}
