use std::net::SocketAddr;
use std::sync::Arc;

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use sea_orm_migration::MigratorTrait;
use tokio::net::TcpListener;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

use bus_travel_backend::{
    config::Config,
    db,
    entities::user::{self, UserRole},
    routes, AppState,
};

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bus_travel_backend=debug,tower_http=debug".into()),
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

    // Create app state
    let state = AppState {
        db,
        config: config.clone(),
    };

    // Configure rate limiting: 100 requests per 60 seconds per IP
    let governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(60)
            .burst_size(100)
            .finish()
            .unwrap(),
    );

    // Create router with middleware
    let app = routes::create_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .layer(GovernorLayer::new(governor_config));

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
    .await
    .expect("Failed to start server");
}

/// Seed the admin account if it doesn't exist
async fn seed_admin(db: &sea_orm::DatabaseConnection) {
    let admin_email = "admin@bustravel.com";

    let existing = user::Entity::find()
        .filter(user::Column::Email.eq(admin_email))
        .one(db)
        .await
        .expect("Failed to check for admin");

    if existing.is_none() {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(b"admin123", &salt)
            .expect("Failed to hash admin password")
            .to_string();

        let admin = user::ActiveModel {
            id: Set(Uuid::new_v4()),
            email: Set(admin_email.to_string()),
            password_hash: Set(password_hash),
            name: Set("Admin".to_string()),
            role: Set(UserRole::Admin),
            ..Default::default()
        };

        admin.insert(db).await.expect("Failed to create admin");
        tracing::info!("Admin account created: {}", admin_email);
    }
}
