use axum::http::Request;
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder,
    key_extractor::KeyExtractor,
    GovernorError, GovernorLayer,
};
use uuid::Uuid;

use crate::entities::user::UserRole;
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

/// Type alias for the governor layer with our custom key extractor
pub type RoleGovernorLayer = GovernorLayer<
    UserIdExtractor,
    governor::middleware::NoOpMiddleware<governor::clock::QuantaInstant>,
    axum::body::Body,
>;

/// Create a GovernorLayer for a specific role
/// - Admin: 1000 requests per minute (10x base)
/// - Driver: 500 requests per minute (5x base)  
/// - Traveller: 100 requests per minute (1x base)
pub fn create_role_governor(role: UserRole) -> RoleGovernorLayer {
    let (per_ms, burst) = match role {
        UserRole::Admin => (60, 1000),      // 1000 per minute
        UserRole::Driver => (120, 500),     // 500 per minute
        UserRole::Traveller => (600, 100),  // 100 per minute
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
