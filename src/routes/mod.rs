use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};

use crate::handlers::{admin, auth, driver, traveller};
use crate::middleware::auth::{auth_middleware, require_admin, require_driver, require_traveller};
use crate::AppState;

pub fn create_router(state: AppState) -> Router {
    // Public routes
    let auth_routes = Router::new()
        .route("/register", post(auth::register))
        .route("/login", post(auth::login));

    // Public journey routes (list available journeys, cities)
    let public_routes = Router::new()
        .route("/journeys", get(traveller::list_journeys))
        .route("/journeys/{id}", get(traveller::get_journey))
        .route("/cities", get(traveller::list_cities));

    // Admin routes (requires auth + admin role)
    let admin_routes = Router::new()
        .route("/journeys", get(admin::list_journeys))
        .route("/journeys", post(admin::create_journey))
        .route("/journeys/{id}", put(admin::update_journey))
        .route("/journeys/{id}", delete(admin::delete_journey))
        .route("/journeys/{id}/assign-driver", post(admin::assign_driver))
        .route("/journeys/{id}/passengers", get(admin::journey_passengers))
        .route("/drivers", get(admin::list_drivers))
        .route("/drivers", post(admin::create_driver))
        .route("/drivers/{id}", delete(admin::delete_driver))
        .route("/bookings", get(admin::list_all_bookings))
        .layer(middleware::from_fn(require_admin))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    // Driver routes (requires auth + driver role)
    let driver_routes = Router::new()
        .route("/journeys", get(driver::my_journeys))
        .route("/journeys/{id}/passengers", get(driver::journey_passengers))
        .layer(middleware::from_fn(require_driver))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    // Traveller routes (requires auth + traveller role)
    let traveller_routes = Router::new()
        .route("/", post(traveller::create_booking))
        .route("/", get(traveller::my_bookings))
        .route("/{id}", delete(traveller::cancel_booking))
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
