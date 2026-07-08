//! Event type filtering used by raw shred transaction parsing.
//!
//! Migrated from sol-parser-sdk `grpc::types` without the Yellowstone/gRPC
//! subscription types.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    PumpFun,
    PumpSwap,
    PumpFees,
    RaydiumLaunchlab,
    RaydiumCpmm,
    RaydiumClmm,
    RaydiumAmmV4,
    OrcaWhirlpool,
    MeteoraPools,
    MeteoraDammV2,
    MeteoraDlmm,
    MeteoraDbc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventType {
    // Block events
    BlockMeta,

    // RaydiumLaunchlab events
    RaydiumLaunchlabTrade,
    RaydiumLaunchlabPoolCreate,
    RaydiumLaunchlabMigrateAmm,

    // PumpFun events
    PumpFunTrade,         // All trade events (backward compatible)
    PumpFunBuy,           // Buy events only (filter by ix_name)
    PumpFunSell,          // Sell events only (filter by ix_name)
    PumpFunBuyExactSolIn, // BuyExactSolIn events only (filter by ix_name)
    PumpFunCreate,
    PumpFunCreateV2, // SPL-22 / Mayhem create
    PumpFunComplete,
    PumpFunMigrate,
    /// Pump fees（`pfeeUx...`，`idls/pump_fees.json` Program data events）
    PumpFeesCreateFeeSharingConfig,
    PumpFeesInitializeFeeConfig,
    PumpFeesResetFeeSharingConfig,
    PumpFeesRevokeFeeSharingAuthority,
    PumpFeesTransferFeeSharingAuthority,
    PumpFeesUpdateAdmin,
    PumpFeesUpdateFeeConfig,
    PumpFeesUpdateFeeShares,
    PumpFeesUpsertFeeTiers,
    /// Pump.fun：`migrateBondingCurveCreatorEvent`
    PumpFunMigrateBondingCurveCreator,

    // PumpSwap events
    PumpSwapTrade,
    PumpSwapBuy,
    PumpSwapSell,
    PumpSwapCreatePool,
    PumpSwapLiquidityAdded,
    PumpSwapLiquidityRemoved,
    // PumpSwapPoolUpdated,
    // PumpSwapFeesClaimed,

    // Raydium CPMM events
    RaydiumCpmmSwap,
    RaydiumCpmmDeposit,
    RaydiumCpmmWithdraw,
    RaydiumCpmmInitialize,

    // Raydium CLMM events
    RaydiumClmmSwap,
    RaydiumClmmCreatePool,
    RaydiumClmmOpenPosition,
    RaydiumClmmClosePosition,
    RaydiumClmmIncreaseLiquidity,
    RaydiumClmmDecreaseLiquidity,
    RaydiumClmmLiquidityChange,
    RaydiumClmmConfigChange,
    RaydiumClmmCreatePersonalPosition,
    RaydiumClmmLiquidityCalculate,
    RaydiumClmmOpenLimitOrder,
    RaydiumClmmIncreaseLimitOrder,
    RaydiumClmmDecreaseLimitOrder,
    RaydiumClmmSettleLimitOrder,
    RaydiumClmmUpdateRewardInfos,
    RaydiumClmmOpenPositionWithTokenExtNft,
    RaydiumClmmCollectFee,

    // Raydium AMM V4 events
    RaydiumAmmV4Swap,
    RaydiumAmmV4Deposit,
    RaydiumAmmV4Withdraw,
    RaydiumAmmV4Initialize2,
    RaydiumAmmV4WithdrawPnl,

    // Orca Whirlpool events
    OrcaWhirlpoolSwap,
    OrcaWhirlpoolLiquidityIncreased,
    OrcaWhirlpoolLiquidityDecreased,
    OrcaWhirlpoolPoolInitialized,

    // Meteora events
    MeteoraPoolsSwap,
    MeteoraPoolsAddLiquidity,
    MeteoraPoolsRemoveLiquidity,
    MeteoraPoolsBootstrapLiquidity,
    MeteoraPoolsPoolCreated,
    MeteoraPoolsSetPoolFees,

    // Meteora DAMM V2 events
    MeteoraDammV2Swap,
    MeteoraDammV2AddLiquidity,
    MeteoraDammV2RemoveLiquidity,
    MeteoraDammV2InitializePool,
    MeteoraDammV2CreatePosition,
    MeteoraDammV2ClosePosition,
    // MeteoraDammV2ClaimPositionFee,
    // MeteoraDammV2InitializeReward,
    // MeteoraDammV2FundReward,
    // MeteoraDammV2ClaimReward,

    // Meteora DBC events
    MeteoraDbcSwap,
    MeteoraDbcInitializePool,
    MeteoraDbcCurveComplete,

    // Meteora DLMM events
    MeteoraDlmmSwap,
    MeteoraDlmmAddLiquidity,
    MeteoraDlmmRemoveLiquidity,
    MeteoraDlmmInitializePool,
    MeteoraDlmmInitializeBinArray,
    MeteoraDlmmCreatePosition,
    MeteoraDlmmClosePosition,
    MeteoraDlmmClaimFee,

    // Account events
    TokenAccount,
    TokenInfo,
    NonceAccount,
    AccountPumpFunGlobal,
    AccountPumpFunBondingCurve,
    AccountPumpFunFeeConfig,
    AccountPumpFunSharingConfig,
    AccountPumpFunGlobalVolumeAccumulator,
    AccountPumpFunUserVolumeAccumulator,

    AccountPumpSwapGlobalConfig,
    AccountPumpSwapPool,
    AccountRaydiumClmmAmmConfig,
    AccountRaydiumClmmPoolState,
    AccountRaydiumClmmTickArrayState,
    AccountRaydiumCpmmAmmConfig,
    AccountRaydiumCpmmPoolState,
    AccountOrcaWhirlpool,
    AccountOrcaPosition,
    AccountOrcaTickArray,
    AccountOrcaFeeTier,
    AccountOrcaWhirlpoolsConfig,
}

#[derive(Debug, Clone)]
pub struct EventTypeFilter {
    pub include_only: Option<Vec<EventType>>,
    pub exclude_types: Option<Vec<EventType>>,
}

impl EventTypeFilter {
    pub fn include_only(types: Vec<EventType>) -> Self {
        Self {
            include_only: Some(types),
            exclude_types: None,
        }
    }

    pub fn exclude_types(types: Vec<EventType>) -> Self {
        Self {
            include_only: None,
            exclude_types: Some(types),
        }
    }

    #[inline]
    fn includes_any(&self, event_types: &[EventType]) -> bool {
        event_types
            .iter()
            .any(|event_type| self.should_include(*event_type))
    }

    pub fn should_include(&self, event_type: EventType) -> bool {
        if let Some(ref include_only) = self.include_only {
            // Direct match
            if include_only.contains(&event_type) {
                return true;
            }
            if matches!(
                event_type,
                EventType::PumpFunBuy | EventType::PumpFunSell | EventType::PumpFunBuyExactSolIn
            ) {
                if pumpfun_trade_filter_is_generic(include_only) {
                    return true;
                }
                if event_type == EventType::PumpFunBuyExactSolIn
                    && pumpfun_buy_filter_is_generic(include_only)
                {
                    return true;
                }
                return false;
            }
            if is_pumpfun_create_family(event_type) {
                return include_only.iter().any(|t| is_pumpfun_create_family(*t));
            }
            if matches!(event_type, EventType::PumpSwapBuy | EventType::PumpSwapSell) {
                return include_only.contains(&EventType::PumpSwapTrade);
            }
            return false;
        }

        if let Some(ref exclude_types) = self.exclude_types {
            if exclude_types.contains(&event_type) {
                return false;
            }
            if matches!(
                event_type,
                EventType::PumpFunBuy | EventType::PumpFunSell | EventType::PumpFunBuyExactSolIn
            ) && exclude_types.contains(&EventType::PumpFunTrade)
            {
                return false;
            }
            if event_type == EventType::PumpFunBuyExactSolIn
                && exclude_types.contains(&EventType::PumpFunBuy)
            {
                return false;
            }
            if is_pumpfun_create_family(event_type)
                && exclude_types.iter().any(|t| is_pumpfun_create_family(*t))
            {
                return false;
            }
            if matches!(event_type, EventType::PumpSwapBuy | EventType::PumpSwapSell)
                && exclude_types.contains(&EventType::PumpSwapTrade)
            {
                return false;
            }
            return true;
        }

        true
    }

    pub fn should_include_dex_event(&self, event: &crate::core::events::DexEvent) -> bool {
        let Some(event_type) = event_type_from_dex_event(event) else {
            return true;
        };
        self.should_include(event_type)
    }

    #[inline]
    pub fn includes_block_meta(&self) -> bool {
        if let Some(ref include_only) = self.include_only {
            return include_only.contains(&EventType::BlockMeta);
        }
        false
    }

    #[inline]
    pub fn normalize_dex_event(
        &self,
        event: crate::core::events::DexEvent,
    ) -> crate::core::events::DexEvent {
        use crate::core::events::DexEvent;

        let Some(ref include_only) = self.include_only else {
            return event;
        };
        if pumpfun_trade_filter_is_generic(include_only) {
            return match event {
                DexEvent::PumpFunBuy(t)
                | DexEvent::PumpFunSell(t)
                | DexEvent::PumpFunBuyExactSolIn(t) => DexEvent::PumpFunTrade(t),
                other => other,
            };
        }
        if pumpfun_buy_filter_is_generic(include_only) {
            return match event {
                DexEvent::PumpFunBuyExactSolIn(t) => DexEvent::PumpFunBuy(t),
                other => other,
            };
        }

        event
    }

    #[inline]
    pub fn includes_pumpfun(&self) -> bool {
        self.includes_any(&[
            EventType::PumpFunTrade,
            EventType::PumpFunBuy,
            EventType::PumpFunSell,
            EventType::PumpFunBuyExactSolIn,
            EventType::PumpFunCreate,
            EventType::PumpFunCreateV2,
            EventType::PumpFunComplete,
            EventType::PumpFunMigrate,
            EventType::PumpFunMigrateBondingCurveCreator,
        ])
    }

    #[inline]
    pub fn includes_meteora_damm_v2(&self) -> bool {
        self.includes_any(&[
            EventType::MeteoraDammV2Swap,
            EventType::MeteoraDammV2AddLiquidity,
            EventType::MeteoraDammV2CreatePosition,
            EventType::MeteoraDammV2ClosePosition,
            EventType::MeteoraDammV2InitializePool,
            EventType::MeteoraDammV2RemoveLiquidity,
        ])
    }

    #[inline]
    pub fn includes_pump_fees(&self) -> bool {
        self.includes_any(&[
            EventType::PumpFeesCreateFeeSharingConfig,
            EventType::PumpFeesInitializeFeeConfig,
            EventType::PumpFeesResetFeeSharingConfig,
            EventType::PumpFeesRevokeFeeSharingAuthority,
            EventType::PumpFeesTransferFeeSharingAuthority,
            EventType::PumpFeesUpdateAdmin,
            EventType::PumpFeesUpdateFeeConfig,
            EventType::PumpFeesUpdateFeeShares,
            EventType::PumpFeesUpsertFeeTiers,
        ])
    }

    /// Check if PumpSwap protocol events are included in the filter
    #[inline]
    pub fn includes_pumpswap(&self) -> bool {
        self.includes_any(&[
            EventType::PumpSwapTrade,
            EventType::PumpSwapBuy,
            EventType::PumpSwapSell,
            EventType::PumpSwapCreatePool,
            EventType::PumpSwapLiquidityAdded,
            EventType::PumpSwapLiquidityRemoved,
        ])
    }

    /// Check if Raydium LaunchLab events are included in the filter.
    #[inline]
    pub fn includes_raydium_launchlab(&self) -> bool {
        self.includes_any(&[
            EventType::RaydiumLaunchlabTrade,
            EventType::RaydiumLaunchlabPoolCreate,
            EventType::RaydiumLaunchlabMigrateAmm,
        ])
    }

    #[inline]
    pub fn includes_raydium_cpmm(&self) -> bool {
        self.includes_any(&[
            EventType::RaydiumCpmmSwap,
            EventType::RaydiumCpmmDeposit,
            EventType::RaydiumCpmmWithdraw,
            EventType::RaydiumCpmmInitialize,
        ])
    }

    #[inline]
    pub fn includes_raydium_clmm(&self) -> bool {
        self.includes_any(&[
            EventType::RaydiumClmmSwap,
            EventType::RaydiumClmmCreatePool,
            EventType::RaydiumClmmOpenPosition,
            EventType::RaydiumClmmClosePosition,
            EventType::RaydiumClmmIncreaseLiquidity,
            EventType::RaydiumClmmDecreaseLiquidity,
            EventType::RaydiumClmmLiquidityChange,
            EventType::RaydiumClmmConfigChange,
            EventType::RaydiumClmmCreatePersonalPosition,
            EventType::RaydiumClmmLiquidityCalculate,
            EventType::RaydiumClmmOpenLimitOrder,
            EventType::RaydiumClmmIncreaseLimitOrder,
            EventType::RaydiumClmmDecreaseLimitOrder,
            EventType::RaydiumClmmSettleLimitOrder,
            EventType::RaydiumClmmUpdateRewardInfos,
            EventType::RaydiumClmmOpenPositionWithTokenExtNft,
            EventType::RaydiumClmmCollectFee,
        ])
    }

    #[inline]
    pub fn includes_raydium_amm_v4(&self) -> bool {
        self.includes_any(&[
            EventType::RaydiumAmmV4Swap,
            EventType::RaydiumAmmV4Deposit,
            EventType::RaydiumAmmV4Withdraw,
            EventType::RaydiumAmmV4Initialize2,
            EventType::RaydiumAmmV4WithdrawPnl,
        ])
    }

    #[inline]
    pub fn includes_orca_whirlpool(&self) -> bool {
        self.includes_any(&[
            EventType::OrcaWhirlpoolSwap,
            EventType::OrcaWhirlpoolLiquidityIncreased,
            EventType::OrcaWhirlpoolLiquidityDecreased,
            EventType::OrcaWhirlpoolPoolInitialized,
        ])
    }

    #[inline]
    pub fn includes_meteora_pools(&self) -> bool {
        self.includes_any(&[
            EventType::MeteoraPoolsSwap,
            EventType::MeteoraPoolsAddLiquidity,
            EventType::MeteoraPoolsRemoveLiquidity,
            EventType::MeteoraPoolsBootstrapLiquidity,
            EventType::MeteoraPoolsPoolCreated,
            EventType::MeteoraPoolsSetPoolFees,
        ])
    }

    #[inline]
    pub fn includes_meteora_dlmm(&self) -> bool {
        self.includes_any(&[
            EventType::MeteoraDlmmSwap,
            EventType::MeteoraDlmmAddLiquidity,
            EventType::MeteoraDlmmRemoveLiquidity,
            EventType::MeteoraDlmmInitializePool,
            EventType::MeteoraDlmmInitializeBinArray,
            EventType::MeteoraDlmmCreatePosition,
            EventType::MeteoraDlmmClosePosition,
            EventType::MeteoraDlmmClaimFee,
        ])
    }

    #[inline]
    pub fn includes_meteora_dbc(&self) -> bool {
        self.includes_any(&[
            EventType::MeteoraDbcSwap,
            EventType::MeteoraDbcInitializePool,
            EventType::MeteoraDbcCurveComplete,
        ])
    }
}

#[inline]
fn pumpfun_trade_filter_is_generic(include_only: &[EventType]) -> bool {
    include_only.contains(&EventType::PumpFunTrade)
        && !include_only.iter().any(|t| {
            matches!(
                t,
                EventType::PumpFunBuy | EventType::PumpFunSell | EventType::PumpFunBuyExactSolIn
            )
        })
}

#[inline]
fn pumpfun_buy_filter_is_generic(include_only: &[EventType]) -> bool {
    include_only.contains(&EventType::PumpFunBuy)
        && !include_only.contains(&EventType::PumpFunBuyExactSolIn)
}

#[inline]
fn is_pumpfun_create_family(event_type: EventType) -> bool {
    matches!(
        event_type,
        EventType::PumpFunCreate | EventType::PumpFunCreateV2
    )
}

#[inline]
pub fn event_type_from_dex_event(event: &crate::core::events::DexEvent) -> Option<EventType> {
    use crate::core::events::DexEvent;
    match event {
        DexEvent::PumpFunCreate(_) => Some(EventType::PumpFunCreate),
        DexEvent::PumpFunCreateV2(_) => Some(EventType::PumpFunCreateV2),
        DexEvent::PumpFunTrade(_) => Some(EventType::PumpFunTrade),
        DexEvent::PumpFunBuy(_) => Some(EventType::PumpFunBuy),
        DexEvent::PumpFunSell(_) => Some(EventType::PumpFunSell),
        DexEvent::PumpFunBuyExactSolIn(_) => Some(EventType::PumpFunBuyExactSolIn),
        DexEvent::PumpFunMigrate(_) => Some(EventType::PumpFunMigrate),
        DexEvent::PumpFeesCreateFeeSharingConfig(_) => {
            Some(EventType::PumpFeesCreateFeeSharingConfig)
        }
        DexEvent::PumpFeesInitializeFeeConfig(_) => Some(EventType::PumpFeesInitializeFeeConfig),
        DexEvent::PumpFeesResetFeeSharingConfig(_) => {
            Some(EventType::PumpFeesResetFeeSharingConfig)
        }
        DexEvent::PumpFeesRevokeFeeSharingAuthority(_) => {
            Some(EventType::PumpFeesRevokeFeeSharingAuthority)
        }
        DexEvent::PumpFeesTransferFeeSharingAuthority(_) => {
            Some(EventType::PumpFeesTransferFeeSharingAuthority)
        }
        DexEvent::PumpFeesUpdateAdmin(_) => Some(EventType::PumpFeesUpdateAdmin),
        DexEvent::PumpFeesUpdateFeeConfig(_) => Some(EventType::PumpFeesUpdateFeeConfig),
        DexEvent::PumpFeesUpdateFeeShares(_) => Some(EventType::PumpFeesUpdateFeeShares),
        DexEvent::PumpFeesUpsertFeeTiers(_) => Some(EventType::PumpFeesUpsertFeeTiers),
        DexEvent::PumpFunMigrateBondingCurveCreator(_) => {
            Some(EventType::PumpFunMigrateBondingCurveCreator)
        }
        DexEvent::PumpFunGlobalAccount(_) => Some(EventType::AccountPumpFunGlobal),
        DexEvent::PumpFunBondingCurveAccount(_) => Some(EventType::AccountPumpFunBondingCurve),
        DexEvent::PumpFunFeeConfigAccount(_) => Some(EventType::AccountPumpFunFeeConfig),
        DexEvent::PumpFunSharingConfigAccount(_) => Some(EventType::AccountPumpFunSharingConfig),
        DexEvent::PumpFunGlobalVolumeAccumulatorAccount(_) => {
            Some(EventType::AccountPumpFunGlobalVolumeAccumulator)
        }
        DexEvent::PumpFunUserVolumeAccumulatorAccount(_) => {
            Some(EventType::AccountPumpFunUserVolumeAccumulator)
        }
        DexEvent::PumpSwapTrade(_) => Some(EventType::PumpSwapTrade),
        DexEvent::PumpSwapBuy(_) => Some(EventType::PumpSwapBuy),
        DexEvent::PumpSwapSell(_) => Some(EventType::PumpSwapSell),
        DexEvent::PumpSwapCreatePool(_) => Some(EventType::PumpSwapCreatePool),
        DexEvent::PumpSwapLiquidityAdded(_) => Some(EventType::PumpSwapLiquidityAdded),
        DexEvent::PumpSwapLiquidityRemoved(_) => Some(EventType::PumpSwapLiquidityRemoved),
        DexEvent::MeteoraDammV2Swap(_) => Some(EventType::MeteoraDammV2Swap),
        DexEvent::MeteoraDammV2CreatePosition(_) => Some(EventType::MeteoraDammV2CreatePosition),
        DexEvent::MeteoraDammV2ClosePosition(_) => Some(EventType::MeteoraDammV2ClosePosition),
        DexEvent::MeteoraDammV2AddLiquidity(_) => Some(EventType::MeteoraDammV2AddLiquidity),
        DexEvent::MeteoraDammV2RemoveLiquidity(_) => Some(EventType::MeteoraDammV2RemoveLiquidity),
        DexEvent::MeteoraDammV2InitializePool(_) => Some(EventType::MeteoraDammV2InitializePool),
        DexEvent::MeteoraDbcSwap(_) => Some(EventType::MeteoraDbcSwap),
        DexEvent::MeteoraDbcInitializePool(_) => Some(EventType::MeteoraDbcInitializePool),
        DexEvent::MeteoraDbcCurveComplete(_) => Some(EventType::MeteoraDbcCurveComplete),
        DexEvent::RaydiumLaunchlabTrade(_) => Some(EventType::RaydiumLaunchlabTrade),
        DexEvent::RaydiumLaunchlabPoolCreate(_) => Some(EventType::RaydiumLaunchlabPoolCreate),
        DexEvent::RaydiumLaunchlabMigrateAmm(_) => Some(EventType::RaydiumLaunchlabMigrateAmm),
        DexEvent::RaydiumClmmSwap(_) => Some(EventType::RaydiumClmmSwap),
        DexEvent::RaydiumClmmCreatePool(_) => Some(EventType::RaydiumClmmCreatePool),
        DexEvent::RaydiumClmmOpenPosition(_) => Some(EventType::RaydiumClmmOpenPosition),
        DexEvent::RaydiumClmmOpenPositionWithTokenExtNft(_) => {
            Some(EventType::RaydiumClmmOpenPositionWithTokenExtNft)
        }
        DexEvent::RaydiumClmmClosePosition(_) => Some(EventType::RaydiumClmmClosePosition),
        DexEvent::RaydiumClmmIncreaseLiquidity(_) => Some(EventType::RaydiumClmmIncreaseLiquidity),
        DexEvent::RaydiumClmmDecreaseLiquidity(_) => Some(EventType::RaydiumClmmDecreaseLiquidity),
        DexEvent::RaydiumClmmLiquidityChange(_) => Some(EventType::RaydiumClmmLiquidityChange),
        DexEvent::RaydiumClmmConfigChange(_) => Some(EventType::RaydiumClmmConfigChange),
        DexEvent::RaydiumClmmCreatePersonalPosition(_) => {
            Some(EventType::RaydiumClmmCreatePersonalPosition)
        }
        DexEvent::RaydiumClmmLiquidityCalculate(_) => {
            Some(EventType::RaydiumClmmLiquidityCalculate)
        }
        DexEvent::RaydiumClmmOpenLimitOrder(_) => Some(EventType::RaydiumClmmOpenLimitOrder),
        DexEvent::RaydiumClmmIncreaseLimitOrder(_) => {
            Some(EventType::RaydiumClmmIncreaseLimitOrder)
        }
        DexEvent::RaydiumClmmDecreaseLimitOrder(_) => {
            Some(EventType::RaydiumClmmDecreaseLimitOrder)
        }
        DexEvent::RaydiumClmmSettleLimitOrder(_) => Some(EventType::RaydiumClmmSettleLimitOrder),
        DexEvent::RaydiumClmmUpdateRewardInfos(_) => Some(EventType::RaydiumClmmUpdateRewardInfos),
        DexEvent::RaydiumClmmCollectFee(_) => Some(EventType::RaydiumClmmCollectFee),
        DexEvent::RaydiumClmmAmmConfigAccount(_) => Some(EventType::AccountRaydiumClmmAmmConfig),
        DexEvent::RaydiumClmmPoolStateAccount(_) => Some(EventType::AccountRaydiumClmmPoolState),
        DexEvent::RaydiumClmmTickArrayStateAccount(_) => {
            Some(EventType::AccountRaydiumClmmTickArrayState)
        }
        DexEvent::RaydiumCpmmSwap(_) => Some(EventType::RaydiumCpmmSwap),
        DexEvent::RaydiumCpmmDeposit(_) => Some(EventType::RaydiumCpmmDeposit),
        DexEvent::RaydiumCpmmWithdraw(_) => Some(EventType::RaydiumCpmmWithdraw),
        DexEvent::RaydiumCpmmInitialize(_) => Some(EventType::RaydiumCpmmInitialize),
        DexEvent::RaydiumCpmmAmmConfigAccount(_) => Some(EventType::AccountRaydiumCpmmAmmConfig),
        DexEvent::RaydiumCpmmPoolStateAccount(_) => Some(EventType::AccountRaydiumCpmmPoolState),
        DexEvent::RaydiumAmmV4Swap(_) => Some(EventType::RaydiumAmmV4Swap),
        DexEvent::RaydiumAmmV4Deposit(_) => Some(EventType::RaydiumAmmV4Deposit),
        DexEvent::RaydiumAmmV4Initialize2(_) => Some(EventType::RaydiumAmmV4Initialize2),
        DexEvent::RaydiumAmmV4Withdraw(_) => Some(EventType::RaydiumAmmV4Withdraw),
        DexEvent::RaydiumAmmV4WithdrawPnl(_) => Some(EventType::RaydiumAmmV4WithdrawPnl),
        DexEvent::OrcaWhirlpoolSwap(_) => Some(EventType::OrcaWhirlpoolSwap),
        DexEvent::OrcaWhirlpoolLiquidityIncreased(_) => {
            Some(EventType::OrcaWhirlpoolLiquidityIncreased)
        }
        DexEvent::OrcaWhirlpoolLiquidityDecreased(_) => {
            Some(EventType::OrcaWhirlpoolLiquidityDecreased)
        }
        DexEvent::OrcaWhirlpoolPoolInitialized(_) => Some(EventType::OrcaWhirlpoolPoolInitialized),
        DexEvent::OrcaWhirlpoolAccount(_) => Some(EventType::AccountOrcaWhirlpool),
        DexEvent::OrcaPositionAccount(_) => Some(EventType::AccountOrcaPosition),
        DexEvent::OrcaTickArrayAccount(_) => Some(EventType::AccountOrcaTickArray),
        DexEvent::OrcaFeeTierAccount(_) => Some(EventType::AccountOrcaFeeTier),
        DexEvent::OrcaWhirlpoolsConfigAccount(_) => Some(EventType::AccountOrcaWhirlpoolsConfig),
        DexEvent::MeteoraPoolsSwap(_) => Some(EventType::MeteoraPoolsSwap),
        DexEvent::MeteoraPoolsAddLiquidity(_) => Some(EventType::MeteoraPoolsAddLiquidity),
        DexEvent::MeteoraPoolsRemoveLiquidity(_) => Some(EventType::MeteoraPoolsRemoveLiquidity),
        DexEvent::MeteoraPoolsBootstrapLiquidity(_) => {
            Some(EventType::MeteoraPoolsBootstrapLiquidity)
        }
        DexEvent::MeteoraPoolsPoolCreated(_) => Some(EventType::MeteoraPoolsPoolCreated),
        DexEvent::MeteoraPoolsSetPoolFees(_) => Some(EventType::MeteoraPoolsSetPoolFees),
        DexEvent::MeteoraDlmmSwap(_) => Some(EventType::MeteoraDlmmSwap),
        DexEvent::MeteoraDlmmAddLiquidity(_) => Some(EventType::MeteoraDlmmAddLiquidity),
        DexEvent::MeteoraDlmmRemoveLiquidity(_) => Some(EventType::MeteoraDlmmRemoveLiquidity),
        DexEvent::MeteoraDlmmInitializePool(_) => Some(EventType::MeteoraDlmmInitializePool),
        DexEvent::MeteoraDlmmInitializeBinArray(_) => {
            Some(EventType::MeteoraDlmmInitializeBinArray)
        }
        DexEvent::MeteoraDlmmCreatePosition(_) => Some(EventType::MeteoraDlmmCreatePosition),
        DexEvent::MeteoraDlmmClosePosition(_) => Some(EventType::MeteoraDlmmClosePosition),
        DexEvent::MeteoraDlmmClaimFee(_) => Some(EventType::MeteoraDlmmClaimFee),
        DexEvent::TokenAccount(_) => Some(EventType::TokenAccount),
        DexEvent::TokenInfo(_) => Some(EventType::TokenInfo),
        DexEvent::NonceAccount(_) => Some(EventType::NonceAccount),
        DexEvent::PumpSwapGlobalConfigAccount(_) => Some(EventType::AccountPumpSwapGlobalConfig),
        DexEvent::PumpSwapPoolAccount(_) => Some(EventType::AccountPumpSwapPool),
        DexEvent::BlockMeta(_) => Some(EventType::BlockMeta),
        DexEvent::Error(_) => None,
    }
}
