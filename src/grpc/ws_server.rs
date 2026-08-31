use super::WS_SENDER;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{interval_at, timeout, Duration, Instant};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn run_ws_server(addr: &str) {
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(error) => {
            log::error!("failed to bind WebSocket server at {addr}: {error}");
            return;
        }
    };
    log::info!("WebSocket server listening at {addr}");

    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(connection) => connection,
            Err(error) => {
                log::warn!("WebSocket accept failed: {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let mut rx = WS_SENDER.subscribe();
        tokio::spawn(async move {
            let ws_stream = match timeout(HANDSHAKE_TIMEOUT, accept_async(stream)).await {
                Ok(Ok(ws_stream)) => ws_stream,
                Ok(Err(error)) => {
                    log::debug!("WebSocket handshake failed for {peer_addr}: {error}");
                    return;
                }
                Err(_) => {
                    log::debug!("WebSocket handshake timed out for {peer_addr}");
                    return;
                }
            };
            let (mut ws_sender, mut ws_receiver) = ws_stream.split();
            let ping_period = Duration::from_secs(30);
            let mut ping_interval = interval_at(Instant::now() + ping_period, ping_period);

            loop {
                tokio::select! {
                    incoming = ws_receiver.next() => match incoming {
                        Some(Ok(Message::Close(frame))) => {
                            let _ = ws_sender.send(Message::Close(frame)).await;
                            break;
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            if ws_sender.send(Message::Pong(payload)).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                        Some(Ok(_)) => {}
                        Some(Err(error)) => {
                            log::debug!("WebSocket receive failed for {peer_addr}: {error}");
                            break;
                        }
                    },
                    msg = rx.recv() => match msg {
                        Ok(msg) => {
                            if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(RecvError::Lagged(skipped)) => {
                            log::debug!("WebSocket client {peer_addr} lagged by {skipped} messages");
                        }
                        Err(RecvError::Closed) => break,
                    },
                    _ = ping_interval.tick() => {
                            if ws_sender.send(Message::Ping(vec![].into())).await.is_err() {
                                break;
                            }
                    }
                }
            }
        });
    }
}
