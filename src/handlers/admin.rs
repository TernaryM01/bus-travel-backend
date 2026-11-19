use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::extract::{Json, Path};
use crate::handlers::shared::{self, JourneyPassengersResponse};
use crate::handlers::traveller::CityInfo;
use crate::entities::{booking, city, journey, user};
use crate::entities::journey::JourneyStatus;
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
    pub status: JourneyStatus,
}

#[derive(Debug, Serialize)]
pub struct DriverInfo {
    pub id: Uuid,
    pub name: String,
    pub email: String,
}

// ============ City Management ============

#[derive(Debug, Deserialize)]
pub struct UpdateStationRequest {
    pub station_name: String,
    pub station_lat: f64,
    pub station_lng: f64,
}

/// Maximum station name length, matching the `station_name` column
const MAX_STATION_NAME_CHARS: usize = 100;

/// List all cities
pub async fn list_cities(State(state): State<AppState>) -> AppResult<Json<Vec<CityInfo>>> {
    let cities = city::Entity::find()
        .order_by_asc(city::Column::Id)
        .all(&state.db)
        .await?;

    let responses: Vec<CityInfo> = cities
        .into_iter()
        .map(|c| CityInfo {
            id: c.id,
            name: c.name,
            center_lat: c.center_lat,
            center_lng: c.center_lng,
            pickup_radius_km: c.pickup_radius_km,
            station_name: c.station_name,
            station_lat: c.station_lat,
            station_lng: c.station_lng,
        })
        .collect();

    Ok(Json(responses))
}

/// Change a city's drop-off station (name and location)
pub async fn update_city_station(
    State(state): State<AppState>,
    Path(city_id): Path<i32>,
    Json(payload): Json<UpdateStationRequest>,
) -> AppResult<StatusCode> {
    let station_name = payload.station_name.trim();
    if station_name.is_empty() {
        return Err(AppError::BadRequest("Nama terminal tidak boleh kosong".to_string()));
    }
    if station_name.chars().count() > MAX_STATION_NAME_CHARS {
        return Err(AppError::BadRequest(format!(
            "Nama terminal maksimal {} karakter",
            MAX_STATION_NAME_CHARS
        )));
    }
    if !(-90.0..=90.0).contains(&payload.station_lat)
        || !(-180.0..=180.0).contains(&payload.station_lng)
    {
        return Err(AppError::BadRequest("Koordinat terminal tidak valid".to_string()));
    }

    let city_record = city::Entity::find_by_id(city_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Kota tidak ditemukan".to_string()))?;

    let mut active: city::ActiveModel = city_record.into();
    active.station_name = Set(station_name.to_string());
    active.station_lat = Set(payload.station_lat);
    active.station_lng = Set(payload.station_lng);
    active.update(&state.db).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// List all journeys (admin)
pub async fn list_journeys(State(state): State<AppState>) -> AppResult<Json<Vec<JourneyResponse>>> {
    let journeys = journey::Entity::find()
        .order_by_desc(journey::Column::DepartureTime)
        .order_by_asc(journey::Column::Id)
        .all(&state.db)
        .await?;
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
            status: j.status,
        });
    }

    Ok(Json(responses))
}

/// Create a new journey (admin)
pub async fn create_journey(
    State(state): State<AppState>,
    Json(payload): Json<CreateJourneyRequest>,
) -> AppResult<StatusCode> {
    // Validate cities
    let origin = city::Entity::find_by_id(payload.origin_city_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest("Kota asal tidak valid".to_string()))?;

    let dest = city::Entity::find_by_id(payload.destination_city_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::BadRequest("Kota tujuan tidak valid".to_string()))?;

    if origin.id == dest.id {
        return Err(AppError::BadRequest(
            "Kota asal dan tujuan harus berbeda".to_string(),
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

    journey.insert(&state.db).await?;

    Ok(StatusCode::CREATED)
}

/// Update a journey (admin)
pub async fn update_journey(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateJourneyRequest>,
) -> AppResult<StatusCode> {
    let journey = journey::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Perjalanan tidak ditemukan".to_string()))?;

    let mut active: journey::ActiveModel = journey.into();

    if let Some(origin_id) = payload.origin_city_id {
        city::Entity::find_by_id(origin_id)
            .one(&state.db)
            .await?
            .ok_or_else(|| AppError::BadRequest("Kota asal tidak valid".to_string()))?;
        active.origin_city_id = Set(origin_id);
    }

    if let Some(dest_id) = payload.destination_city_id {
        city::Entity::find_by_id(dest_id)
            .one(&state.db)
            .await?
            .ok_or_else(|| AppError::BadRequest("Kota tujuan tidak valid".to_string()))?;
        active.destination_city_id = Set(dest_id);
    }

    if let Some(time) = payload.departure_time {
        active.departure_time = Set(time.into());
    }

    if let Some(seats) = payload.total_seats {
        active.total_seats = Set(seats);
    }

    active.update(&state.db).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Delete a journey (admin)
pub async fn delete_journey(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let result = journey::Entity::delete_by_id(id).exec(&state.db).await?;

    if result.rows_affected == 0 {
        return Err(AppError::NotFound("Perjalanan tidak ditemukan".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
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
) -> AppResult<StatusCode> {
    // Validate driver exists and has driver role
    let driver = user::Entity::find_by_id(payload.driver_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Sopir tidak ditemukan".to_string()))?;

    if driver.role != UserRole::Driver {
        return Err(AppError::BadRequest("Pengguna ini bukan sopir".to_string()));
    }

    // Get journey
    let journey = journey::Entity::find_by_id(journey_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Perjalanan tidak ditemukan".to_string()))?;

    let mut active: journey::ActiveModel = journey.into();
    active.driver_id = Set(Some(payload.driver_id));

    active.update(&state.db).await?;

    Ok(StatusCode::NO_CONTENT)
}

// ============ User Management ============

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
}

/// List all users (admin)
pub async fn list_all_users(State(state): State<AppState>) -> AppResult<Json<Vec<UserResponse>>> {
    let users = user::Entity::find()
        .order_by_asc(user::Column::CreatedAt)
        .order_by_asc(user::Column::Id)
        .all(&state.db)
        .await?;

    let responses: Vec<UserResponse> = users
        .into_iter()
        .map(|u| UserResponse {
            id: u.id,
            email: u.email,
            name: u.name,
            role: u.role,
            created_at: u.created_at.with_timezone(&Utc),
        })
        .collect();

    Ok(Json(responses))
}

/// List all drivers (admin)
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
        .order_by_asc(user::Column::Name)
        .order_by_asc(user::Column::Id)
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

/// Update user role (admin)
#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub role: UserRole,
}

pub async fn update_user_role(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(payload): Json<UpdateRoleRequest>,
) -> AppResult<StatusCode> {
    let user = user::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Pengguna tidak ditemukan".to_string()))?;

    let old_role = user.role.clone();

    // Handle role change side effects
    if old_role == UserRole::Driver && payload.role != UserRole::Driver {
        // Unassign from all journeys
        let journeys = journey::Entity::find()
            .filter(journey::Column::DriverId.eq(user_id))
            .all(&state.db)
            .await?;
        for j in journeys {
            let mut active: journey::ActiveModel = j.into();
            active.driver_id = Set(None);
            active.update(&state.db).await?;
        }
    }

    if old_role == UserRole::Traveller && payload.role != UserRole::Traveller {
        // Delete all bookings (bookings belong to travellers)
        booking::Entity::delete_many()
            .filter(booking::Column::UserId.eq(user_id))
            .exec(&state.db)
            .await?;
    }

    let mut active: user::ActiveModel = user.into();
    active.role = Set(payload.role.clone());
    active.update(&state.db).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Delete any user account (admin)
pub async fn delete_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let user = user::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Pengguna tidak ditemukan".to_string()))?;

    // Handle cleanup based on role
    if user.role == UserRole::Driver {
        // Unassign from all journeys
        let journeys = journey::Entity::find()
            .filter(journey::Column::DriverId.eq(id))
            .all(&state.db)
            .await?;
        for j in journeys {
            let mut active: journey::ActiveModel = j.into();
            active.driver_id = Set(None);
            active.update(&state.db).await?;
        }
    }

    // Delete user's bookings (if any - travellers will have bookings)
    booking::Entity::delete_many()
        .filter(booking::Column::UserId.eq(id))
        .exec(&state.db)
        .await?;

    // Delete user
    user::Entity::delete_by_id(id).exec(&state.db).await?;

    Ok(StatusCode::NO_CONTENT)
}

// ============ Bookings Management (Admin) ============

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
    let bookings = booking::Entity::find()
        .order_by_asc(booking::Column::CreatedAt)
        .order_by_asc(booking::Column::Id)
        .all(&state.db)
        .await?;
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

/// Delete any booking (admin)
pub async fn delete_booking(
    State(state): State<AppState>,
    Path(booking_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let result = booking::Entity::delete_by_id(booking_id)
        .exec(&state.db)
        .await?;

    if result.rows_affected == 0 {
        return Err(AppError::NotFound("Pesanan tidak ditemukan".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Update booking (admin) - can change pickup point and/or seats
#[derive(Debug, Deserialize)]
pub struct UpdateBookingRequest {
    pub pickup_lat: Option<f64>,
    pub pickup_lng: Option<f64>,
    pub seats: Option<i32>,
}

pub async fn update_booking(
    State(state): State<AppState>,
    Path(booking_id): Path<Uuid>,
    Json(payload): Json<UpdateBookingRequest>,
) -> AppResult<StatusCode> {
    let booking_record = booking::Entity::find_by_id(booking_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Pesanan tidak ditemukan".to_string()))?;

    let mut active: booking::ActiveModel = booking_record.clone().into();

    // Update pickup point (no validation - admin can set any location)
    if payload.pickup_lat.is_some() || payload.pickup_lng.is_some() {
        let new_lat = payload.pickup_lat.unwrap_or(booking_record.pickup_lat);
        let new_lng = payload.pickup_lng.unwrap_or(booking_record.pickup_lng);

        active.pickup_lat = Set(new_lat);
        active.pickup_lng = Set(new_lng);
    }

    // Update seats (admin can overbook)
    if let Some(new_seats) = payload.seats {
        if new_seats <= 0 {
            return Err(AppError::BadRequest("Jumlah kursi harus minimal 1".to_string()));
        }

        active.seats = Set(new_seats);
    }

    active.update(&state.db).await?;

    Ok(StatusCode::NO_CONTENT)
}

// ============ Journey Passengers (for admin view) ============

/// Get passenger pickup points for a specific journey (admin)
/// Unlike the driver version, this doesn't check if the admin is assigned to the journey
pub async fn journey_passengers(
    State(state): State<AppState>,
    Path(journey_id): Path<Uuid>,
) -> AppResult<Json<JourneyPassengersResponse>> {
    // Get the journey
    let journey = journey::Entity::find_by_id(journey_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Perjalanan tidak ditemukan".to_string()))?;

    Ok(Json(shared::journey_passengers(&state.db, journey).await?))
}
