use crate::common::logs_data::{
    BonkCreateTokenInfo, CreateTokenInfo, EventTrait, TradeInfo, TradeRequest, TransferInfo,
};
use base64::engine::general_purpose;
use base64::Engine;

pub const PROGRAM_DATA: &str = "Program data: ";

#[derive(Debug)]
pub enum PumpfunEvent {
    RawInstruction(Box<crate::core::events::RawInstructionEvent>),
    NewToken(CreateTokenInfo),
    // NewDevTrade(TradeInfo),
    NewBonkToken {
        token: BonkCreateTokenInfo,
        trade: TradeRequest,
    },
    NewUserTrade(TradeInfo),
    NewBotTrade(TradeInfo),
    NewToken2 {
        token: CreateTokenInfo,
        trade: TradeInfo,
    },
    // NewTip(TipInfo),
    Error(String),
}

#[derive(Debug)]
pub enum DexEvent {
    NewToken(CreateTokenInfo),
    NewUserTrade(TradeInfo),
    NewBotTrade(TradeInfo),
    Error(String),
}

#[derive(Debug)]
pub enum SystemEvent {
    NewTransfer(TransferInfo),
    Error(String),
}

// #[derive(Debug, Clone, Copy)]
// pub struct PumpEvent {}

impl PumpfunEvent {
    pub fn parse_logs(logs: &[String]) -> (Option<CreateTokenInfo>, Option<TradeInfo>) {
        let mut create_info = None;
        let mut trade_info = None;
        // Compatibility API returns the last event of each kind. For all route
        // legs use LogFilter::parse_instruction instead of collapsing to a tuple.
        if let Ok(events) = crate::common::logs_filters::LogFilter::parse_instruction(logs, None) {
            for event in events {
                match event {
                    crate::common::logs_data::DexInstruction::CreateToken(token) => {
                        create_info = Some(token)
                    }
                    crate::common::logs_data::DexInstruction::UserTrade(trade)
                    | crate::common::logs_data::DexInstruction::BotTrade(trade) => {
                        trade_info = Some(trade)
                    }
                    _ => {}
                }
            }
        }
        (create_info, trade_info)
    }
}

const RAY_LOG_PREFIX: &str = "ray_log: ";

#[inline]
fn ray_log_payload(log: &str) -> Option<&str> {
    let encoded = log.split_once(RAY_LOG_PREFIX)?.1;
    let end = encoded
        .find(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '+' | '/' | '=')
        })
        .unwrap_or(encoded.len());
    (end > 0).then_some(&encoded[..end])
}
#[derive(Debug, Clone, Copy)]
pub struct RaydiumEvent {}

impl RaydiumEvent {
    pub fn parse_logs<T: EventTrait>(logs: &[String]) -> Option<T> {
        logs.iter().rev().find_map(|log| {
            let encoded = ray_log_payload(log)?;
            let bytes = general_purpose::STANDARD.decode(encoded).ok()?;
            T::from_bytes(&bytes).ok()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::error::ClientResult;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct RawEvent(Vec<u8>);

    impl EventTrait for RawEvent {
        fn from_bytes(bytes: &[u8]) -> ClientResult<Self> {
            Ok(Self(bytes.to_vec()))
        }
    }

    #[test]
    fn raydium_parser_returns_latest_valid_log_without_regex() {
        let logs = vec![
            "Program log: ray_log: AQ== trailing".to_owned(),
            "Program log: ray_log: Ag==".to_owned(),
        ];

        assert_eq!(RaydiumEvent::parse_logs(&logs), Some(RawEvent(vec![2])));
    }

    #[test]
    fn malformed_encoded_logs_are_ignored() {
        let raydium_logs = vec!["Program log: ray_log: ===".to_owned()];
        assert_eq!(RaydiumEvent::parse_logs::<RawEvent>(&raydium_logs), None);

        let pump_logs = vec![
            format!("{PROGRAM_DATA}not-base64"),
            format!("{PROGRAM_DATA}AQIDBA=="),
        ];
        assert_eq!(PumpfunEvent::parse_logs(&pump_logs), (None, None));
    }
}
