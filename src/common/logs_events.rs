use crate::common::logs_data::{
    BonkCreateTokenInfo, CreateTokenInfo, EventTrait, TradeInfo, TradeRequest, TransferInfo,
};
use base64::engine::general_purpose;
use base64::Engine;

pub const PROGRAM_DATA: &str = "Program data: ";

#[derive(Debug)]
pub enum PumpfunEvent {
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
        let mut create_info: Option<CreateTokenInfo> = None;
        let mut trade_info: Option<TradeInfo> = None;

        for log in logs.iter().rev() {
            let Some(encoded) = log.strip_prefix(PROGRAM_DATA) else {
                continue;
            };
            let Ok(bytes) = general_purpose::STANDARD.decode(encoded) else {
                continue;
            };
            let Some(payload) = bytes.get(8..) else {
                continue;
            };

            if create_info.is_none() {
                if let Ok(event) = CreateTokenInfo::from_bytes(payload) {
                    create_info = Some(event);
                    continue;
                }
            }

            if trade_info.is_none() {
                if let Ok(event) = TradeInfo::from_bytes(payload) {
                    trade_info = Some(event);
                }
            }
            if create_info.is_some() && trade_info.is_some() {
                break;
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
