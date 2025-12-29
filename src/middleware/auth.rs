use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};

use crate::entities::user::UserRole;
use crate::error::{AppError, AppResult};
use crate::utils::jwt::{verify_token, Claims};
use crate::AppState;

/// Extract and validate JWT token from Authorization header
pub async fn auth_middleware(
    State(state): State<AppState>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    mut request: Request,
    next: Next,
) -> AppResult<Response> {
    let claims = verify_token(auth.token(), &state.config.jwt_secret)?;
    request.extensions_mut().insert(claims);
    Ok(next.run(request).await)
}

/// Require admin role
pub async fn require_admin(
    request: Request,
    next: Next,
) -> AppResult<Response> {
    let claims = request
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("No authentication found".to_string()))?;

    if claims.role != UserRole::Admin {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    Ok(next.run(request).await)
}

/// Require driver role
pub async fn require_driver(
    request: Request,
    next: Next,
) -> AppResult<Response> {
    let claims = request
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("No authentication found".to_string()))?;

    if claims.role != UserRole::Driver {
        return Err(AppError::Forbidden("Driver access required".to_string()));
    }

    Ok(next.run(request).await)
}

/// Require traveller role
pub async fn require_traveller(
    request: Request,
    next: Next,
) -> AppResult<Response> {
    let claims = request
        .extensions()
        .get::<Claims>()
        .ok_or_else(|| AppError::Unauthorized("No authentication found".to_string()))?;

    if claims.role != UserRole::Traveller {
        return Err(AppError::Forbidden("Traveller access required".to_string()));
    }

    Ok(next.run(request).await)
}
