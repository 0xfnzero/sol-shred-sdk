use std::net::SocketAddr;
use std::time::Duration;

use crate::shred::RawShredConfig;

/// Raw ShredStream client configuration.
#[derive(Debug, Clone)]
pub struct ShredStreamConfig {
    pub raw: RawShredConfig,
    pub event_queue_capacity: usize,
    pub reconnect_delay_ms: u64,
    /// Maximum restart attempts after a receive-loop error. `0` means forever.
    pub max_reconnect_attempts: u32,
}

impl Default for ShredStreamConfig {
    fn default() -> Self {
        Self {
            raw: RawShredConfig::default(),
            event_queue_capacity: 100_000,
            reconnect_delay_ms: 250,
            max_reconnect_attempts: 0,
        }
    }
}

impl ShredStreamConfig {
    pub fn low_latency() -> Self {
        Self {
            raw: RawShredConfig {
                reassembly_gap_timeout: Duration::from_millis(250),
                max_tracked_slots: 48,
                ..RawShredConfig::default()
            },
            event_queue_capacity: 100_000,
            reconnect_delay_ms: 50,
            max_reconnect_attempts: 0,
        }
    }

    pub fn high_throughput() -> Self {
        Self {
            raw: RawShredConfig {
                udp_recv_buffer_bytes: 128 * 1024 * 1024,
                max_tracked_slots: 128,
                reassembly_gap_timeout: Duration::from_millis(600),
                ..RawShredConfig::default()
            },
            event_queue_capacity: 500_000,
            reconnect_delay_ms: 250,
            max_reconnect_attempts: 0,
        }
    }

    pub fn with_udp_bind(mut self, udp_bind: SocketAddr) -> Self {
        self.raw.udp_bind = udp_bind;
        self
    }
}
