//! Local HTTP proxy that sits on port 8047 and forwards IDE requests to the Manager.
//!
//! The Antigravity IDE's language_server has a default backend URL.
//! This proxy intercepts all requests, adds the sk-* Bearer token, and forwards
//! them to the Manager (which swaps the token and proxies to Google).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::watch;

/// Configuration for the local proxy
#[derive(Clone)]
pub struct ProxyConfig {
    pub listen_port: u16,
    pub target_url: String,   // e.g. "http://127.0.0.1:8045" or "http://vps:8045"
    pub bearer_token: String, // sk-* token
}

/// Start the local proxy server. Returns a shutdown sender.
/// When the sender is dropped or sends `true`, the server stops.
pub async fn start_proxy(config: ProxyConfig) -> Result<watch::Sender<bool>, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], config.listen_port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Failed to bind to port {}: {}", config.listen_port, e))?;
    eprintln!("[LocalProxy] Bind successful");

    eprintln!("[LocalProxy] Listening on http://{}", addr);
    eprintln!("[LocalProxy] Forwarding to {}", config.target_url);

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let config = Arc::new(config);

    // Create a shared reqwest client for outgoing requests
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300)) // 5 min timeout for long requests
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
    let http_client = Arc::new(http_client);

    tokio::spawn(async move {
        let mut shutdown = shutdown_rx;

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _remote)) => {
                            let config = config.clone();
                            let client = http_client.clone();
                            tokio::spawn(handle_connection(stream, config, client));
                        }
                        Err(e) => {
                            eprintln!("[LocalProxy] Accept error: {}", e);
                        }
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        eprintln!("[LocalProxy] Shutting down");
                        break;
                    }
                }
            }
        }
    });

    Ok(shutdown_tx)
}

/// Handle a single TCP connection
async fn handle_connection(
    stream: tokio::net::TcpStream,
    config: Arc<ProxyConfig>,
    client: Arc<reqwest::Client>,
) {
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;

    let io = TokioIo::new(stream);
    let config = config.clone();
    let client = client.clone();

    let service = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
        let config = config.clone();
        let client = client.clone();
        async move { proxy_request(req, config, client).await }
    });

    if let Err(e) = http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades()
        .await
    {
        if !e.to_string().contains("connection closed") {
            eprintln!("[LocalProxy] Connection error: {}", e);
        }
    }
}

/// Proxy a single request to the Manager
async fn proxy_request(
    req: hyper::Request<hyper::body::Incoming>,
    config: Arc<ProxyConfig>,
    client: Arc<reqwest::Client>,
) -> Result<hyper::Response<http_body_util::Full<bytes::Bytes>>, Infallible> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().map(|q| q.to_string());

    // Intercept userinfo and tokeninfo to satisfy IDE authentication checks
    if path.contains("userinfo") || path.contains("tokeninfo") {
        eprintln!("[LocalProxy] Intercepting auth check: {}", path);
        let mock_json = r#"{
            "id": "12345",
            "email": "proxy_user@antigravity",
            "verified_email": true,
            "picture": "https://lh3.googleusercontent.com/a/default-user",
            "aud": "mock-aud",
            "expires_in": 3600,
            "scope": "https://www.googleapis.com/auth/cloud-platform"
        }"#;

        let resp = hyper::Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .body(http_body_util::Full::new(bytes::Bytes::from(mock_json)))
            .unwrap();
        
        return Ok(resp);
    }

    // Build target URL
    let target_url = if let Some(ref qs) = query {
        format!("{}{}?{}", config.target_url, path, qs)
    } else {
        format!("{}{}", config.target_url, path)
    };

    eprintln!("[LocalProxy] {} {} -> {}", method, uri, target_url);

    // Read the incoming body
    use http_body_util::BodyExt;
    let body_bytes = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            eprintln!("[LocalProxy] Failed to read body: {}", e);
            let resp = hyper::Response::builder()
                .status(502)
                .body(http_body_util::Full::new(bytes::Bytes::from(
                    format!("Failed to read request body: {}", e),
                )))
                .unwrap();
            return Ok(resp);
        }
    };

    // Build outgoing request with Bearer token
    let mut outgoing = client
        .request(
            reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::POST),
            &target_url,
        )
        .header("Authorization", format!("Bearer {}", config.bearer_token));

    // Copy relevant headers (skip host, connection, etc.)
    // We need content-type especially
    outgoing = outgoing.header("Content-Type", "application/json");

    // Send the request
    let response = match outgoing.body(body_bytes.to_vec()).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[LocalProxy] Upstream error: {}", e);
            let resp = hyper::Response::builder()
                .status(502)
                .body(http_body_util::Full::new(bytes::Bytes::from(
                    format!("Upstream error: {}", e),
                )))
                .unwrap();
            return Ok(resp);
        }
    };

    let status = response.status();
    eprintln!("[LocalProxy] Response: {}", status);

    // Build the hyper response
    let mut builder = hyper::Response::builder().status(status.as_u16());

    // Copy response headers
    for (name, value) in response.headers() {
        let name_lower = name.as_str().to_lowercase();
        // Skip hop-by-hop and length-related headers because we reconstruct a Full body
        if name_lower == "transfer-encoding" || name_lower == "content-length" || name_lower == "connection" {
            continue;
        }
        if let Ok(header_name) = hyper::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(header_value) = hyper::header::HeaderValue::from_bytes(value.as_bytes()) {
                builder = builder.header(header_name, header_value);
            }
        }
    }

    // Read response body (for now, read full body — streaming can be added later)
    let resp_bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[LocalProxy] Failed to read response: {}", e);
            bytes::Bytes::from(format!("Failed to read response: {}", e))
        }
    };

    let resp = builder
        .body(http_body_util::Full::new(resp_bytes))
        .unwrap_or_else(|_| {
            hyper::Response::new(http_body_util::Full::new(bytes::Bytes::from("Internal error")))
        });

    Ok(resp)
}
