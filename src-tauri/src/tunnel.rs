use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use futures_util::{StreamExt, SinkExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn start_tunnel_worker(server_url: String, token: String) {
    eprintln!("[Tunnel] Starting tunnel pool with 5 workers");
    for _ in 0..5 {
        let url = server_url.clone();
        let tok = token.clone();
        tokio::spawn(async move {
            tunnel_loop(url, tok).await;
        });
    }
}

async fn tunnel_loop(server_url: String, token: String) {
    let ws_url = server_url.replace("http://", "ws://").replace("https://", "wss://");
    let ws_url = format!("{}/v1/tunnel?token={}", ws_url, token);

    loop {
        match connect_async(&ws_url).await {
            Ok((ws_stream, _)) => {
                eprintln!("[Tunnel] Connected to Manager WebSocket");
                let (mut write, mut read) = ws_stream.split();

                // Wait for CONNECT command
                if let Some(Ok(Message::Text(cmd))) = read.next().await {
                    let cmd_str = cmd.as_str();
                    if cmd_str.starts_with("CONNECT ") {
                        let parts: Vec<&str> = cmd_str.split_whitespace().collect();
                        if parts.len() == 3 {
                            let host = parts[1];
                            let port: u16 = parts[2].parse().unwrap_or(443);
                            eprintln!("[Tunnel] Manager requested connection to {}:{}", host, port);

                            // Use custom DNS (xbox-dns.ru) to resolve host
                            let resolver = crate::dns::create_custom_resolver();
                            let ip = match resolver.lookup_ip(host).await {
                                Ok(response) => {
                                    if let Some(ip) = response.iter().next() {
                                        ip
                                    } else {
                                        let _ = write.send(Message::Text("ERROR No IP found".to_string().into())).await;
                                        continue;
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[Tunnel] DNS resolution failed: {}", e);
                                    let _ = write.send(Message::Text(format!("ERROR DNS failed: {}", e).into())).await;
                                    continue;
                                }
                            };

                            match TcpStream::connect(std::net::SocketAddr::new(ip, port)).await {
                                Ok(mut target_stream) => {
                                    let _ = write.send(Message::Text("CONNECTED".to_string().into())).await;
                                    eprintln!("[Tunnel] Target connected, bridging bytes...");

                                    // Bridge WS and TCP
                                    let (mut target_read, mut target_write) = target_stream.into_split();
                                    
                                    let mut ws_to_tcp = tokio::spawn(async move {
                                        while let Some(Ok(Message::Binary(data))) = read.next().await {
                                            if target_write.write_all(&data).await.is_err() {
                                                break;
                                            }
                                        }
                                    });

                                    let mut tcp_to_ws = tokio::spawn(async move {
                                        let mut buf = vec![0u8; 8192];
                                        loop {
                                            match target_read.read(&mut buf).await {
                                                Ok(0) => break,
                                                Ok(n) => {
                                                    if write.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() {
                                                        break;
                                                    }
                                                }
                                                Err(_) => break,
                                            }
                                        }
                                    });

                                    tokio::select! {
                                        _ = &mut ws_to_tcp => {},
                                        _ = &mut tcp_to_ws => {},
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[Tunnel] Failed to connect to target: {}", e);
                                    let _ = write.send(Message::Text("ERROR".to_string().into())).await;
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("[Tunnel] Failed to connect: {}. Retrying in 5s...", e);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
