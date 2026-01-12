use axum::{
    body::Body,
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};

/// Type alias for the global governor layer (IP-based rate limiting)
pub type GlobalGovernorLayer = GovernorLayer<
    tower_governor::key_extractor::PeerIpKeyExtractor,
    governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>,
    Body,
>;

/// Create a GovernorLayer for global rate limiting (per IP address)
/// - 1000 requests per minute (one token every 60ms)
/// - Applied before authentication to protect against DDoS
pub fn create_global_governor() -> GlobalGovernorLayer {
    let config = Arc::new(
        GovernorConfigBuilder::default()
            .per_millisecond(60) // One token every 60ms (1000 per minute)
            .burst_size(1000)    // Max capacity of the "window"
            .finish()
            .unwrap(),
    );

    GovernorLayer::new(config)
}

/// Middleware to log rate limiting and request details
pub async fn log_request(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let version = request.version();
    
    tracing::debug!(
        client_ip = %addr.ip(),
        method = %method,
        uri = %uri,
        version = ?version,
        "Incoming request"
    );

    let response = next.run(request).await;
    let status = response.status();

    // Log rate limiting specifically
    if status == StatusCode::TOO_MANY_REQUESTS {
        tracing::warn!(
            client_ip = %addr.ip(),
            method = %method,
            uri = %uri,
            status = %status,
            "RATE LIMITED - Request rejected due to too many requests"
        );
    } else if status.is_client_error() || status.is_server_error() {
        tracing::warn!(
            client_ip = %addr.ip(),
            method = %method,
            uri = %uri,
            status = %status,
            "Request failed"
        );
    } else {
        tracing::debug!(
            client_ip = %addr.ip(),
            method = %method,
            uri = %uri,
            status = %status,
            "Request completed"
        );
    }

    response
}
