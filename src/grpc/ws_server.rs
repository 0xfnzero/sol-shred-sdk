use super::WS_SENDER;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::time::{interval, Duration};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

pub async fn run_ws_server(addr: &str) {
    let listener = TcpListener::bind(addr).await.unwrap();
    println!("WebSocket 服务器监听: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        let mut rx = WS_SENDER.subscribe();
        tokio::spawn(async move {
            let ws_stream = accept_async(stream).await.unwrap();
            let (mut ws_sender, mut ws_receiver) = ws_stream.split();

            // 创建一个心跳任务
            let mut ping_interval = interval(Duration::from_secs(30));

            // 处理接收消息的任务
            let receive_task = tokio::spawn(async move {
                while let Some(Ok(_msg)) = ws_receiver.next().await {
                    // 可以处理客户端发来的消息
                }
            });

            // 处理发送消息和心跳的任务
            let send_task = tokio::spawn(async move {
                loop {
                    tokio::select! {
                        // 发送广播消息
                        msg = rx.recv() => {
                            if let Ok(msg) = msg {
                                if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                        // 发送心跳
                        _ = ping_interval.tick() => {
                            if ws_sender.send(Message::Ping(vec![].into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });

            // 等待任意一个任务结束
            tokio::select! {
                _ = receive_task => {},
                _ = send_task => {},
            }
        });
    }
}
