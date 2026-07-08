use std::net::SocketAddr;
use std::time::Duration;

/// Configuration for raw UDP shred ingestion and reassembly.
#[derive(Debug, Clone)]
pub struct RawShredConfig {
    /// UDP address to bind when using [`crate::shred::RawShredClient`].
    pub udp_bind: SocketAddr,
    /// Kernel receive buffer target. Set to `0` to leave the OS default.
    pub udp_recv_buffer_bytes: usize,
    /// Maximum UDP datagram size read by the client.
    pub max_datagram_size: usize,
    /// Number of incomplete slots kept in memory.
    pub max_tracked_slots: usize,
    /// Timeout for incomplete slot buffers.
    pub reassembly_gap_timeout: Duration,
    /// Drop packets older than the highest emitted slot.
    pub forward_slot_watermark: bool,
    /// Bytes to skip before the Solana shred. Keep `0` for native raw shreds.
    pub udp_payload_prefix_skip: usize,
    /// Guardrail for deshred output before bincode decoding.
    pub max_deshred_bytes: usize,
}

impl Default for RawShredConfig {
    fn default() -> Self {
        Self {
            udp_bind: "0.0.0.0:8001".parse().expect("valid default UDP bind"),
            udp_recv_buffer_bytes: 64 * 1024 * 1024,
            max_datagram_size: 2048,
            max_tracked_slots: 64,
            reassembly_gap_timeout: Duration::from_millis(400),
            forward_slot_watermark: false,
            udp_payload_prefix_skip: 0,
            max_deshred_bytes: 16 * 1024 * 1024,
        }
    }
}

impl RawShredConfig {
    pub fn with_udp_bind(mut self, udp_bind: SocketAddr) -> Self {
        self.udp_bind = udp_bind;
        self
    }

    pub fn with_forward_slot_watermark(mut self, enabled: bool) -> Self {
        self.forward_slot_watermark = enabled;
        self
    }
}
