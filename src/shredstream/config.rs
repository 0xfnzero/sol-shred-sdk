use std::net::SocketAddr;
use std::time::Duration;

use crate::shred::RawShredConfig;

/// Jito-style ShredStream gRPC source configuration.
#[derive(Debug, Clone)]
pub struct JitoShredStreamConfig {
    pub endpoint: String,
    pub channel_size: usize,
}

impl JitoShredStreamConfig {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            channel_size: 1_000,
        }
    }

    pub fn with_channel_size(mut self, channel_size: usize) -> Self {
        self.channel_size = channel_size.max(1);
        self
    }
}

/// Input and shred/entry decode mode.
#[derive(Debug, Clone)]
pub enum ShredDecodeMode {
    /// Native raw Solana UDP shreds decoded locally with `solana-ledger`.
    RawUdp(RawShredConfig),
    /// Jito-style ShredStream gRPC entries. This source is retained for
    /// compatibility; it still feeds the same transaction/event parser.
    JitoGrpc(JitoShredStreamConfig),
}

impl Default for ShredDecodeMode {
    fn default() -> Self {
        Self::RawUdp(RawShredConfig::default())
    }
}

impl ShredDecodeMode {
    pub fn raw_udp(config: RawShredConfig) -> Self {
        Self::RawUdp(config)
    }

    pub fn jito_grpc(endpoint: impl Into<String>) -> Self {
        Self::JitoGrpc(JitoShredStreamConfig::new(endpoint))
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::RawUdp(_) => "raw-udp",
            Self::JitoGrpc(_) => "jito-grpc",
        }
    }
}

/// Multi-source ShredStream client configuration.
#[derive(Debug, Clone)]
pub struct ShredStreamConfig {
    pub decode_mode: ShredDecodeMode,
    pub event_queue_capacity: usize,
    pub reconnect_delay_ms: u64,
    /// Maximum restart attempts after a receive-loop error. `0` means forever.
    pub max_reconnect_attempts: u32,
}

impl Default for ShredStreamConfig {
    fn default() -> Self {
        Self {
            decode_mode: ShredDecodeMode::default(),
            event_queue_capacity: 100_000,
            reconnect_delay_ms: 250,
            max_reconnect_attempts: 0,
        }
    }
}

impl ShredStreamConfig {
    pub fn low_latency() -> Self {
        Self {
            decode_mode: ShredDecodeMode::RawUdp(RawShredConfig {
                reassembly_gap_timeout: Duration::from_millis(250),
                max_tracked_slots: 48,
                ..RawShredConfig::default()
            }),
            event_queue_capacity: 100_000,
            reconnect_delay_ms: 50,
            max_reconnect_attempts: 0,
        }
    }

    pub fn high_throughput() -> Self {
        Self {
            decode_mode: ShredDecodeMode::RawUdp(RawShredConfig {
                udp_recv_buffer_bytes: 128 * 1024 * 1024,
                max_tracked_slots: 128,
                reassembly_gap_timeout: Duration::from_millis(600),
                ..RawShredConfig::default()
            }),
            event_queue_capacity: 500_000,
            reconnect_delay_ms: 250,
            max_reconnect_attempts: 0,
        }
    }

    pub fn jito_grpc(endpoint: impl Into<String>) -> Self {
        Self {
            decode_mode: ShredDecodeMode::jito_grpc(endpoint),
            event_queue_capacity: 100_000,
            reconnect_delay_ms: 250,
            max_reconnect_attempts: 0,
        }
    }

    pub fn with_decode_mode(mut self, decode_mode: ShredDecodeMode) -> Self {
        self.decode_mode = decode_mode;
        self
    }

    pub fn with_udp_bind(mut self, udp_bind: SocketAddr) -> Self {
        if let ShredDecodeMode::RawUdp(raw) = &mut self.decode_mode {
            raw.udp_bind = udp_bind;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_decode_mode_is_raw_udp() {
        let config = ShredStreamConfig::default();
        assert!(matches!(config.decode_mode, ShredDecodeMode::RawUdp(_)));
    }

    #[test]
    fn jito_decode_mode_keeps_endpoint() {
        let config = ShredStreamConfig::jito_grpc("http://127.0.0.1:10000");
        match config.decode_mode {
            ShredDecodeMode::JitoGrpc(jito) => {
                assert_eq!(jito.endpoint, "http://127.0.0.1:10000");
            }
            ShredDecodeMode::RawUdp(_) => panic!("expected jito grpc mode"),
        }
    }

    #[test]
    fn udp_bind_only_applies_to_raw_mode() {
        let udp_bind: SocketAddr = "127.0.0.1:9001".parse().unwrap();
        let raw = ShredStreamConfig::default().with_udp_bind(udp_bind);
        match raw.decode_mode {
            ShredDecodeMode::RawUdp(raw) => assert_eq!(raw.udp_bind, udp_bind),
            ShredDecodeMode::JitoGrpc(_) => panic!("expected raw udp mode"),
        }

        let jito = ShredStreamConfig::jito_grpc("http://127.0.0.1:10000").with_udp_bind(udp_bind);
        match jito.decode_mode {
            ShredDecodeMode::JitoGrpc(jito) => {
                assert_eq!(jito.endpoint, "http://127.0.0.1:10000");
            }
            ShredDecodeMode::RawUdp(_) => panic!("expected jito grpc mode"),
        }
    }
}
