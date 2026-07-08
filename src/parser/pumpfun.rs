use std::num::NonZeroUsize;

use lru::LruCache;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;

use crate::common::logs_data::{
    BonkCreateTokenInfo, CreateTokenInfo, DexInstruction, TradeInfo, TradeRequest,
};
use crate::common::logs_events::PumpfunEvent;
use crate::common::logs_filters::LogFilter;
use crate::common::AnyResult;
use crate::parser::TransactionEventParser;

#[derive(Debug, Clone)]
pub struct PumpfunParserConfig {
    pub bot_wallet: Option<Pubkey>,
    pub dedup_capacity: NonZeroUsize,
}

impl Default for PumpfunParserConfig {
    fn default() -> Self {
        Self {
            bot_wallet: None,
            dedup_capacity: NonZeroUsize::new(100_000).expect("non-zero dedup capacity"),
        }
    }
}

impl PumpfunParserConfig {
    pub fn with_bot_wallet(mut self, bot_wallet: Option<Pubkey>) -> Self {
        self.bot_wallet = bot_wallet;
        self
    }
}

/// PumpFun / Raydium LaunchLab parser for decoded Solana transactions.
pub struct PumpfunEventParser {
    config: PumpfunParserConfig,
    processed: LruCache<Signature, ()>,
}

impl PumpfunEventParser {
    pub fn new(config: PumpfunParserConfig) -> Self {
        Self {
            processed: LruCache::new(config.dedup_capacity),
            config,
        }
    }

    #[inline]
    pub fn clear_dedup(&mut self) {
        self.processed.clear();
    }

    pub fn process_transaction<F>(
        &mut self,
        transaction: &VersionedTransaction,
        slot: u64,
        mut callback: F,
    ) -> AnyResult<usize>
    where
        F: FnMut(PumpfunEvent),
    {
        let Some(signature) = transaction.signatures.first().cloned() else {
            return Ok(0);
        };

        if self.processed.put(signature, ()).is_some() {
            return Ok(0);
        }

        let instructions =
            LogFilter::parse_compiled_instruction(transaction, self.config.bot_wallet)?;
        let mut emitted = 0usize;

        let mut token_info: Option<CreateTokenInfo> = None;
        let mut dev_trade_info: Option<TradeInfo> = None;
        let mut bonk_token_info: Option<BonkCreateTokenInfo> = None;
        let mut bonk_trade_info: Option<TradeRequest> = None;

        for instruction in instructions {
            match instruction {
                DexInstruction::CreateToken(mut token) => {
                    let (limit, price, fee_merchant, fee) = LogFilter::parse_tip_info(transaction);
                    token.slot = slot;
                    token.unit_limit = limit.unwrap_or(0);
                    token.unit_price = price.unwrap_or(0);
                    token.fee_merchant = fee_merchant.unwrap_or_default();
                    token.fee = fee.unwrap_or(0);
                    token_info = Some(token);
                }
                DexInstruction::BonkCreateToken(mut token) => {
                    let (limit, price, fee_merchant, fee) = LogFilter::parse_tip_info(transaction);
                    token.unit_limit = limit.unwrap_or(0);
                    token.unit_price = price.unwrap_or(0);
                    token.fee_merchant = fee_merchant.unwrap_or_default();
                    token.fee = fee.unwrap_or(0);
                    bonk_token_info = Some(token);
                }
                DexInstruction::BonkTrade(trade_request) => {
                    bonk_trade_info = Some(trade_request);
                }
                DexInstruction::UserTrade(mut trade) => {
                    trade.slot = slot;
                    if token_info.is_some() {
                        merge_trade(&mut dev_trade_info, trade);
                    } else {
                        callback(PumpfunEvent::NewUserTrade(trade));
                        emitted += 1;
                    }
                }
                DexInstruction::BotTrade(mut trade) => {
                    trade.slot = slot;
                    callback(PumpfunEvent::NewBotTrade(trade));
                    emitted += 1;
                }
                _ => {}
            }
        }

        match (token_info, dev_trade_info, bonk_token_info, bonk_trade_info) {
            (Some(token), Some(trade), None, None) => {
                callback(PumpfunEvent::NewToken2 { token, trade });
                emitted += 1;
            }
            (Some(token), None, None, None) => {
                callback(PumpfunEvent::NewToken(token));
                emitted += 1;
            }
            (None, None, Some(token), Some(trade)) => {
                callback(PumpfunEvent::NewBonkToken { token, trade });
                emitted += 1;
            }
            _ => {}
        }

        Ok(emitted)
    }
}

impl Default for PumpfunEventParser {
    fn default() -> Self {
        Self::new(PumpfunParserConfig::default())
    }
}

impl TransactionEventParser for PumpfunEventParser {
    type Event = PumpfunEvent;

    #[inline]
    fn parse_transaction_events<F>(
        &mut self,
        transaction: &VersionedTransaction,
        slot: u64,
        _tx_index: u64,
        _recv_us: i64,
        emit: F,
    ) -> AnyResult<usize>
    where
        F: FnMut(Self::Event),
    {
        self.process_transaction(transaction, slot, emit)
    }
}

fn merge_trade(target: &mut Option<TradeInfo>, trade: TradeInfo) {
    if let Some(existing) = target {
        existing.sol_amount = existing.sol_amount.saturating_add(trade.sol_amount);
        existing.token_amount = existing.token_amount.saturating_add(trade.token_amount);
    } else {
        *target = Some(trade);
    }
}
