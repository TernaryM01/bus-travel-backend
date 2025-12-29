use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

use std::net::SocketAddr;

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
