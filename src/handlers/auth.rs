use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{Json, extract::State};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppState;
use crate::entities::user::{self, UserRole};
use crate::error::{AppError, AppResult};
use crate::utils::jwt::create_token;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct GoogleLoginRequest {
    pub credential: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserInfo,
}

#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: UserRole,
}

/// Register a new traveller account
pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> AppResult<Json<AuthResponse>> {
    // Check if email already exists
    let existing = user::Entity::find()
        .filter(user::Column::Email.eq(&payload.email))
        .one(&state.db)
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict("Email already registered".to_string()));
    }

    // Hash password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(payload.password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?
        .to_string();

    // Create user
    let user_id = Uuid::new_v4();
    let new_user = user::ActiveModel {
        id: Set(user_id),
        email: Set(payload.email.clone()),
        password_hash: Set(Some(password_hash)),
        google_id: Set(None),
        name: Set(payload.name.clone()),
        role: Set(UserRole::Traveller),
        ..Default::default()
    };

    let user = new_user.insert(&state.db).await?;

    // Generate token
    let token = create_token(
        user.id,
        &user.email,
        user.role.clone(),
        &state.config.jwt_secret,
        state.config.jwt_expiration_hours,
    )?;

    Ok(Json(AuthResponse {
        token,
        user: UserInfo {
            id: user.id,
            email: user.email,
            name: user.name,
            role: user.role,
        },
    }))
}

/// Login with email and password
pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> AppResult<Json<AuthResponse>> {
    // Find user by email
    let user = user::Entity::find()
        .filter(user::Column::Email.eq(&payload.email))
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid email or password".to_string()))?;

    // Verify password (handle Google-only accounts)
    let password_hash = user.password_hash.as_ref().ok_or_else(|| {
        AppError::Unauthorized(
            "This account uses Google login. Please sign in with Google.".to_string(),
        )
    })?;

    let parsed_hash = PasswordHash::new(password_hash)
        .map_err(|e| AppError::Internal(format!("Failed to parse password hash: {}", e)))?;

    Argon2::default()
        .verify_password(payload.password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized("Invalid email or password".to_string()))?;

    // Generate token
    let token = create_token(
        user.id,
        &user.email,
        user.role.clone(),
        &state.config.jwt_secret,
        state.config.jwt_expiration_hours,
    )?;

    Ok(Json(AuthResponse {
        token,
        user: UserInfo {
            id: user.id,
            email: user.email,
            name: user.name,
            role: user.role,
        },
    }))
}

#[derive(Debug, Deserialize)]
struct GoogleTokenInfo {
    sub: String,
    email: String,
    name: Option<String>,
    aud: String,
}

/// Login/Register with Google
pub async fn google_login(
    State(state): State<AppState>,
    Json(payload): Json<GoogleLoginRequest>,
) -> AppResult<Json<AuthResponse>> {
    // Verify token with Google
    let client = reqwest::Client::new();
    let resp = client
        .get("https://oauth2.googleapis.com/tokeninfo")
        .query(&[("id_token", &payload.credential)])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to verify Google token: {}", e)))?;

    if !resp.status().is_success() {
        return Err(AppError::Unauthorized("Invalid Google token".to_string()));
    }

    let token_info: GoogleTokenInfo = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse Google response: {}", e)))?;

    // Verify audience matches our client ID
    if token_info.aud != state.config.oauth_client_id {
        return Err(AppError::Unauthorized("Invalid token audience".to_string()));
    }

    // Find existing user by google_id or email
    let existing_user = user::Entity::find()
        .filter(
            user::Column::GoogleId
                .eq(&token_info.sub)
                .or(user::Column::Email.eq(&token_info.email)),
        )
        .one(&state.db)
        .await?;

    let user = match existing_user {
        Some(u) => {
            // Link Google account if user exists by email but not google_id
            if u.google_id.is_none() {
                let mut am: user::ActiveModel = u.into();
                am.google_id = Set(Some(token_info.sub));
                am.update(&state.db).await?
            } else {
                u
            }
        }
        None => {
            // Create new user
            let new_user = user::ActiveModel {
                id: Set(Uuid::new_v4()),
                email: Set(token_info.email),
                password_hash: Set(None),
                google_id: Set(Some(token_info.sub)),
                name: Set(token_info.name.unwrap_or_else(|| "Google User".to_string())),
                role: Set(UserRole::Traveller),
                ..Default::default()
            };
            new_user.insert(&state.db).await?
        }
    };

    // Generate JWT token
    let token = create_token(
        user.id,
        &user.email,
        user.role.clone(),
        &state.config.jwt_secret,
        state.config.jwt_expiration_hours,
    )?;

    Ok(Json(AuthResponse {
        token,
        user: UserInfo {
            id: user.id,
            email: user.email,
            name: user.name,
            role: user.role,
        },
    }))
}
