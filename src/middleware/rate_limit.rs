use axum::body::Body;
use axum::http::{Response, StatusCode};
use axum::response::IntoResponse;
use std::sync::Arc;
use tower_governor::{governor::GovernorConfigBuilder, GovernorError, GovernorLayer};

/// Type alias for the global governor layer (IP-based rate limiting)
pub type GlobalGovernorLayer = GovernorLayer<
    tower_governor::key_extractor::PeerIpKeyExtractor,
    governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>,
    Body,
>;

/// Error handler for rate limiting - logs the rejection and returns a 429 response.
/// This function is used by both global and role-based rate limiters.
pub fn rate_limit_error_handler(err: GovernorError) -> Response<Body> {
    match err {
        GovernorError::TooManyRequests { .. } => {
            tracing::warn!(
                status = %StatusCode::TOO_MANY_REQUESTS,
                "Rate limited - request rejected due to too many requests"
            );
            (StatusCode::TOO_MANY_REQUESTS, "Too Many Requests").into_response()
        }
        _ => {
            tracing::error!(
                status = %StatusCode::INTERNAL_SERVER_ERROR,
                "Rate limiter error"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

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

    GovernorLayer::new(config).error_handler(rate_limit_error_handler)
}

/// Create a GovernorLayer for public endpoints (per IP address, with traveller-level limits)
/// - 100 requests per minute (one token every 600ms)
/// - Applied to public routes where there's no authenticated user
/// - Uses same restrictive limits as traveller rate limiting
pub fn create_public_governor() -> GlobalGovernorLayer {
    let config = Arc::new(
        GovernorConfigBuilder::default()
            .per_millisecond(600) // One token every 600ms (100 per minute)
            .burst_size(100)      // Same as traveller limit
            .finish()
            .unwrap(),
    );

    GovernorLayer::new(config).error_handler(rate_limit_error_handler)
}
