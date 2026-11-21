use std::net::SocketAddr;

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use sea_orm_migration::MigratorTrait;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

use bus_travel_backend::{
    AppState,
    config::Config,
    db,
    entities::user::{self, UserRole},
    middleware::rate_limit::create_global_governor,
    routes,
};

#[tokio::main]
async fn main() {
    // Load .env before anything reads env vars
    dotenvy::dotenv().ok();

    // Initialize tracing; the filter is read from the RUST_LOG env var
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_env("RUST_LOG")
                .unwrap_or_else(|_| {
                    let level = if cfg!(debug_assertions) { "debug" } else { "info" };
                    level.into()
                }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = Config::from_env();
    tracing::info!("Starting server at {}", config.server_addr());

    // Connect to database
    let db = db::connect(&config)
        .await
        .expect("Failed to connect to database");
    tracing::info!("Connected to database");

    // Run migrations
    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");
    tracing::info!("Migrations complete");

    // Seed admin account if not exists
    seed_admin(&db).await;

    // Put the read cache in front of the database. Built after migrations so it
    // sees the final schema (tables, foreign keys, triggers).
    let db = db::CachedDb::new(db, config.db_cache_max_mb * 1024 * 1024)
        .await
        .expect("Failed to set up database cache");
    tracing::info!("Database cache ready ({} MB cap)", config.db_cache_max_mb);

    // Create app state
    let state = AppState {
        db,
        config: config.clone(),
    };

    // Create router with middleware
    let app = routes::create_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(create_global_governor());

    // Start server with socket address for rate limiting
    let addr: SocketAddr = config.server_addr().parse().expect("Invalid address");
    let listener = TcpListener::bind(addr)
        .await
        .expect("Failed to bind to address");

    tracing::info!("Server listening on {}", addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("Failed to start server");

    tracing::info!("Server stopped");
}

/// Resolves when the server should stop: Ctrl+C in a terminal, or SIGTERM,
/// which hosting platforms send before stopping or redeploying the app.
/// The server then stops accepting connections, lets requests already in
/// progress finish, and exits normally.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for Ctrl+C");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to listen for SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutting down: finishing requests in progress");
}

/// Seed the admin account if no admins exist
async fn seed_admin(db: &sea_orm::DatabaseConnection) {
    let admin_email = "admin@bustravel.com";

    let any_admin = user::Entity::find()
        .filter(user::Column::Role.eq(UserRole::Admin))
        .one(db)
        .await
        .expect("Failed to check for admin");

    if any_admin.is_some() {
        return;
    }

    // No admin exists.
    // If a non-admin account already uses the default admin email, don't promote it.
    let email_taken = user::Entity::find()
        .filter(user::Column::Email.eq(admin_email))
        .one(db)
        .await
        .expect("Failed to check for existing account")
        .is_some();

    if email_taken {
        tracing::error!(
            "No admin account exists, and {} is already used by a non-admin account; \
             skipping default admin creation. Promote an account to admin directly in the database.",
            admin_email
        );
        return;
    }

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(b"admin123", &salt)
        .expect("Failed to hash admin password")
        .to_string();

    let admin = user::ActiveModel {
        id: Set(Uuid::new_v4()),
        email: Set(admin_email.to_string()),
        password_hash: Set(Some(password_hash)),
        google_id: Set(None),
        name: Set("Admin".to_string()),
        role: Set(UserRole::Admin),
        ..Default::default()
    };

    admin.insert(db).await.expect("Failed to create admin");
    tracing::info!("Admin account created: {}", admin_email);
}
