use axum::http::Request;
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder,
    key_extractor::KeyExtractor,
    GovernorError, GovernorLayer,
};
use uuid::Uuid;

use crate::middleware::rate_limit::rate_limit_error_handler;
use crate::utils::jwt::Claims;

/// Custom key extractor that extracts user ID from JWT claims in request extensions
#[derive(Debug, Clone, Copy)]
pub struct UserIdExtractor;

impl KeyExtractor for UserIdExtractor {
    type Key = Uuid;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        // Get claims from request extensions (set by auth_middleware)
        let claims = req
            .extensions()
            .get::<Claims>()
            .ok_or(GovernorError::UnableToExtractKey)?;

        Ok(claims.sub)
    }
}

pub type RoleGovernorLayer = GovernorLayer<
    UserIdExtractor,
    governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>,
    axum::body::Body,
>;

/// Create a GovernorLayer for a specific role
/// - Admin: No rate limiting (10x base in IP-based global rate limiter)
/// - Driver: 500 requests per minute (5x base)
/// - Traveller: 100 requests per minute (base)

// The dedicated roles enum for rate limiting is meant to
// prevent the role-based rate limiter from being used in admin routes.
pub enum RateLimitedRole {
    Traveller,
    Driver,
}
// impl RateLimitedRole {
//     fn from_user_role(role: UserRole) -> Option<Self> {
//         match role {
//             UserRole::Traveller => Some(RateLimitedRole::Traveller),
//             UserRole::Driver => Some(RateLimitedRole::Driver),
//             UserRole::Admin => None,
//         }
//     }
//     ...
// }

pub fn create_role_governor(role: RateLimitedRole) -> RoleGovernorLayer {
    let (per_ms, burst) = match role {
        RateLimitedRole::Driver => (120 * 2, 500),     // 500 / 2 per minute
        RateLimitedRole::Traveller => (600 * 2, 100),  // 100 / 2 per minute
    };

    let config = Arc::new(
        GovernorConfigBuilder::default()
            .per_millisecond(per_ms)
            .burst_size(burst)
            .key_extractor(UserIdExtractor)
            .finish()
            .unwrap(),
    );

    GovernorLayer::new(config).error_handler(rate_limit_error_handler)
}
