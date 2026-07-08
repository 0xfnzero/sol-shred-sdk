use std::net::UdpSocket as StdUdpSocket;
use std::time::Instant;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

use super::config::RawShredConfig;
use super::decoder::{entries_to_tx_batch, ShredEntryBatch, ShredTxBatch};
use super::error::ShredResult;
use super::reassembler::{RawShredDecoder, ShredDecoderStats};

/// Async UDP client for raw Solana shreds.
pub struct RawShredClient {
    socket: UdpSocket,
    decoder: RawShredDecoder,
    config: RawShredConfig,
}

impl RawShredClient {
    pub async fn bind(config: RawShredConfig) -> ShredResult<Self> {
        let socket = bind_udp_socket(&config)?;
        let decoder = RawShredDecoder::new(config.clone());

        Ok(Self {
            socket,
            decoder,
            config,
        })
    }

    #[inline]
    pub fn stats(&self) -> ShredDecoderStats {
        self.decoder.stats()
    }

    #[inline]
    pub fn decoder_mut(&mut self) -> &mut RawShredDecoder {
        &mut self.decoder
    }

    /// Run the receive loop and callback on each completed Entry batch.
    pub async fn run_entries<F>(&mut self, mut callback: F) -> ShredResult<()>
    where
        F: FnMut(ShredEntryBatch) + Send,
    {
        let mut buf = vec![0u8; self.config.max_datagram_size.max(1280)];

        loop {
            let n = self.socket.recv(&mut buf).await?;
            let now = Instant::now();
            let batches = self.decoder.push_packet(&buf[..n], now);
            self.decoder.evict_stale_slots(now);

            for batch in batches {
                callback(batch);
            }
        }
    }

    /// Run the receive loop and callback on each completed transaction batch.
    pub async fn run_transactions<F>(&mut self, mut callback: F) -> ShredResult<()>
    where
        F: FnMut(ShredTxBatch) + Send,
    {
        self.run_entries(|batch| callback(entries_to_tx_batch(batch)))
            .await
    }
}

fn bind_udp_socket(config: &RawShredConfig) -> ShredResult<UdpSocket> {
    let socket = Socket::new(
        Domain::for_address(config.udp_bind),
        Type::DGRAM,
        Some(Protocol::UDP),
    )?;
    socket.set_reuse_address(true)?;
    if config.udp_recv_buffer_bytes > 0 {
        socket.set_recv_buffer_size(config.udp_recv_buffer_bytes)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&config.udp_bind.into())?;

    let std_socket: StdUdpSocket = socket.into();
    Ok(UdpSocket::from_std(std_socket)?)
}
