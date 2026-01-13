use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};

use crate::middleware::role_rate_limit::RateLimitedRole;
use crate::handlers::{admin, auth, driver, traveller};
use crate::middleware::auth::{auth_middleware, require_admin, require_driver, require_traveller};
use crate::middleware::rate_limit::create_public_governor;
use crate::middleware::role_rate_limit::create_role_governor;
use crate::AppState;

pub fn create_router(state: AppState) -> Router {
    // Create role-specific governor layers
    let driver_governor = create_role_governor(RateLimitedRole::Driver);
    let traveller_governor = create_role_governor(RateLimitedRole::Traveller);
    // Create IP-based governor for public routes (with traveller-level limits)
    let public_governor = create_public_governor();

    // Public routes (with traveller-level rate limiting per IP)
    let auth_routes = Router::new()
        .route("/register", post(auth::register))
        .route("/login", post(auth::login))
        .layer(public_governor.clone());

    // Public journey routes (list available journeys, cities)
    let public_routes = Router::new()
        .route("/journeys", get(traveller::list_journeys))
        .route("/journeys/{id}", get(traveller::get_journey))
        .route("/cities", get(traveller::list_cities))
        .layer(public_governor);

    // Admin routes (requires auth + admin role)
    // Rate limit: 1000 requests per minute (10x base)
    let admin_routes = Router::new()
        // Journey management
        .route("/journeys", get(admin::list_journeys))
        .route("/journeys", post(admin::create_journey))
        .route("/journeys/{id}", put(admin::update_journey))
        .route("/journeys/{id}", delete(admin::delete_journey))
        .route("/journeys/{id}/assign-driver", post(admin::assign_driver))
        .route("/journeys/{id}/passengers", get(admin::journey_passengers))
        // User management
        .route("/users", get(admin::list_all_users))
        .route("/users/{id}", delete(admin::delete_user))
        .route("/users/{id}/role", put(admin::update_user_role))
        // Drivers
        .route("/drivers", get(admin::list_drivers))
        // Booking management
        .route("/bookings", get(admin::list_all_bookings))
        .route("/bookings/{id}", delete(admin::delete_booking))
        .route("/bookings/{id}", put(admin::update_booking))
        // .layer(admin_governor)  // No need for second rate limiter for admin
        .layer(middleware::from_fn(require_admin))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    // Driver routes (requires auth + driver role)
    // Rate limit: 500 requests per minute (5x base)
    let driver_routes = Router::new()
        .route("/journeys", get(driver::my_journeys))
        .route("/journeys/{id}/passengers", get(driver::journey_passengers))
        .layer(driver_governor)
        .layer(middleware::from_fn(require_driver))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    // Traveller routes (requires auth + traveller role)
    // Rate limit: 100 requests per minute (1x base)
    let traveller_routes = Router::new()
        .route("/", post(traveller::create_booking))
        .route("/", get(traveller::my_bookings))
        .route("/{id}", delete(traveller::cancel_booking))
        .layer(traveller_governor)
        .layer(middleware::from_fn(require_traveller))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    // Combine all routes
    Router::new()
        .nest("/api/auth", auth_routes)
        .nest("/api", public_routes)
        .nest("/api/admin", admin_routes)
        .nest("/api/driver", driver_routes)
        .nest("/api/bookings", traveller_routes)
        .with_state(state)
}
