use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::{booking, city, journey, user};
use crate::entities::user::UserRole;
use crate::error::{AppError, AppResult};
use crate::AppState;

// ============ Journey Management ============

#[derive(Debug, Deserialize)]
pub struct CreateJourneyRequest {
    pub origin_city_id: i32,
    pub destination_city_id: i32,
    pub departure_time: DateTime<Utc>,
    pub total_seats: i32,
}

#[derive(Debug, Deserialize)]
pub struct UpdateJourneyRequest {
    pub origin_city_id: Option<i32>,
    pub destination_city_id: Option<i32>,
    pub departure_time: Option<DateTime<Utc>>,
    pub total_seats: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct JourneyResponse {
    pub id: Uuid,
    pub origin_city: String,
    pub destination_city: String,
    pub departure_time: DateTime<Utc>,
    pub total_seats: i32,
    pub booked_seats: i32,
    pub driver: Option<DriverInfo>,
}

#[derive(Debug, Serialize)]
pub struct DriverInfo {
    pub id: Uuid,
    pub name: String,
    pub email: String,
}

/// List all journeys (admin)
pub async fn list_journeys(State(state): State<AppState>) -> AppResult<Json<Vec<JourneyResponse>>> {
    let journeys = journey::Entity::find().all(&state.db).await?;
    let cities = city::Entity::find().all(&state.db).await?;
    let drivers = user::Entity::find()
        .filter(user::Column::Role.eq(UserRole::Driver))
        .all(&state.db)
        .await?;

    let mut responses = Vec::new();
    for j in journeys {
        let origin = cities.iter().find(|c| c.id == j.origin_city_id);
        let dest = cities.iter().find(|c| c.id == j.destination_city_id);

        let booked: i32 = booking::Entity::find()
            .filter(booking::Column::JourneyId.eq(j.id))
            .all(&state.db)
            .await?
            .iter()
            .map(|b| b.seats)
            .sum();

        let driver = j.driver_id.and_then(|did| {
            drivers.iter().find(|d| d.id == did).map(|d| DriverInfo {
                id: d.id,
                name: d.name.clone(),
                email: d.email.clone(),
            })
        });

        responses.push(JourneyResponse {
            id: j.id,
            origin_city: origin.map(|c| c.name.clone()).unwrap_or_default(),
            destination_city: dest.map(|c| c.name.clone()).unwrap_or_default(),
            departure_time: j.departure_time.with_timezone(&Utc),
            total_seats: j.total_seats,
            booked_seats: booked,
            driver,
        });
    }

    Ok(Json(responses))
}

/// Create a new journey (admin)
pub async fn create_journey(
    State(state): State<AppState>,
    Json(payload): Json<CreateJourneyRequest>,
) -> AppResult<Json<journey::Model>> {
    // Validate cities
    let origin = city::Entity::find_by_id(payload.origin_city_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid origin city".to_string()))?;

    let dest = city::Entity::find_by_id(payload.destination_city_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid destination city".to_string()))?;

    if origin.id == dest.id {
        return Err(AppError::BadRequest(
            "Origin and destination must be different".to_string(),
        ));
    }

    let journey = journey::ActiveModel {
        id: Set(Uuid::new_v4()),
        origin_city_id: Set(payload.origin_city_id),
        destination_city_id: Set(payload.destination_city_id),
        departure_time: Set(payload.departure_time.into()),
        total_seats: Set(payload.total_seats),
        driver_id: Set(None),
        ..Default::default()
    };

    let result = journey.insert(&state.db).await?;
    Ok(Json(result))
}

/// Update a journey (admin)
pub async fn update_journey(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateJourneyRequest>,
) -> AppResult<Json<journey::Model>> {
    let journey = journey::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Journey not found".to_string()))?;

    let mut active: journey::ActiveModel = journey.into();

    if let Some(origin_id) = payload.origin_city_id {
        city::Entity::find_by_id(origin_id)
            .one(&state.db)
            .await?
            .ok_or_else(|| AppError::BadRequest("Invalid origin city".to_string()))?;
        active.origin_city_id = Set(origin_id);
    }

    if let Some(dest_id) = payload.destination_city_id {
        city::Entity::find_by_id(dest_id)
            .one(&state.db)
            .await?
            .ok_or_else(|| AppError::BadRequest("Invalid destination city".to_string()))?;
        active.destination_city_id = Set(dest_id);
    }

    if let Some(time) = payload.departure_time {
        active.departure_time = Set(time.into());
    }

    if let Some(seats) = payload.total_seats {
        active.total_seats = Set(seats);
    }

    let result = active.update(&state.db).await?;
    Ok(Json(result))
}

/// Delete a journey (admin)
pub async fn delete_journey(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let result = journey::Entity::delete_by_id(id).exec(&state.db).await?;

    if result.rows_affected == 0 {
        return Err(AppError::NotFound("Journey not found".to_string()));
    }

    Ok(Json(serde_json::json!({ "message": "Journey deleted" })))
}

/// Assign a driver to a journey (admin)
#[derive(Debug, Deserialize)]
pub struct AssignDriverRequest {
    pub driver_id: Uuid,
}

pub async fn assign_driver(
    State(state): State<AppState>,
    Path(journey_id): Path<Uuid>,
    Json(payload): Json<AssignDriverRequest>,
) -> AppResult<Json<journey::Model>> {
    // Validate driver exists and has driver role
    let driver = user::Entity::find_by_id(payload.driver_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Driver not found".to_string()))?;

    if driver.role != UserRole::Driver {
        return Err(AppError::BadRequest("User is not a driver".to_string()));
    }

    // Get journey
    let journey = journey::Entity::find_by_id(journey_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Journey not found".to_string()))?;

    let mut active: journey::ActiveModel = journey.into();
    active.driver_id = Set(Some(payload.driver_id));

    let result = active.update(&state.db).await?;
    Ok(Json(result))
}

// ============ Driver Management ============

#[derive(Debug, Serialize)]
pub struct DriverResponse {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

/// List all drivers (admin)
pub async fn list_drivers(State(state): State<AppState>) -> AppResult<Json<Vec<DriverResponse>>> {
    let drivers = user::Entity::find()
        .filter(user::Column::Role.eq(UserRole::Driver))
        .all(&state.db)
        .await?;

    let responses: Vec<DriverResponse> = drivers
        .into_iter()
        .map(|d| DriverResponse {
            id: d.id,
            email: d.email,
            name: d.name,
            created_at: d.created_at.with_timezone(&Utc),
        })
        .collect();

    Ok(Json(responses))
}

/// Create a new driver account (admin)
#[derive(Debug, Deserialize)]
pub struct CreateDriverRequest {
    pub email: String,
    pub password: String,
    pub name: String,
}

pub async fn create_driver(
    State(state): State<AppState>,
    Json(payload): Json<CreateDriverRequest>,
) -> AppResult<Json<DriverResponse>> {
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

    let user_id = Uuid::new_v4();
    let new_driver = user::ActiveModel {
        id: Set(user_id),
        email: Set(payload.email.clone()),
        password_hash: Set(password_hash),
        name: Set(payload.name.clone()),
        role: Set(UserRole::Driver),
        ..Default::default()
    };

    let driver = new_driver.insert(&state.db).await?;

    Ok(Json(DriverResponse {
        id: driver.id,
        email: driver.email,
        name: driver.name,
        created_at: driver.created_at.with_timezone(&Utc),
    }))
}

/// Delete a driver account (admin)
pub async fn delete_driver(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    // Verify it's a driver
    let driver = user::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Driver not found".to_string()))?;

    if driver.role != UserRole::Driver {
        return Err(AppError::BadRequest("User is not a driver".to_string()));
    }

    // Unassign driver from journeys first
    let journeys = journey::Entity::find()
        .filter(journey::Column::DriverId.eq(id))
        .all(&state.db)
        .await?;

    for j in journeys {
        let mut active: journey::ActiveModel = j.into();
        active.driver_id = Set(None);
        active.update(&state.db).await?;
    }

    // Delete driver
    user::Entity::delete_by_id(id).exec(&state.db).await?;

    Ok(Json(serde_json::json!({ "message": "Driver deleted" })))
}

// ============ Bookings (for admin view) ============

#[derive(Debug, Serialize)]
pub struct BookingInfo {
    pub id: Uuid,
    pub journey_id: Uuid,
    pub user_name: String,
    pub user_email: String,
    pub seats: i32,
    pub pickup_lat: f64,
    pub pickup_lng: f64,
    pub created_at: DateTime<Utc>,
}

/// List all bookings (admin)
pub async fn list_all_bookings(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<BookingInfo>>> {
    let bookings = booking::Entity::find().all(&state.db).await?;
    let users = user::Entity::find().all(&state.db).await?;

    let responses: Vec<BookingInfo> = bookings
        .into_iter()
        .map(|b| {
            let user = users.iter().find(|u| u.id == b.user_id);
            BookingInfo {
                id: b.id,
                journey_id: b.journey_id,
                user_name: user.map(|u| u.name.clone()).unwrap_or_default(),
                user_email: user.map(|u| u.email.clone()).unwrap_or_default(),
                seats: b.seats,
                pickup_lat: b.pickup_lat,
                pickup_lng: b.pickup_lng,
                created_at: b.created_at.with_timezone(&Utc),
            }
        })
        .collect();

    Ok(Json(responses))
}
