use axum::{extract::State, http::StatusCode, Extension};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::extract::{Json, Path};
use crate::entities::{booking, city, journey};
use crate::error::{AppError, AppResult};
use crate::utils::geo::is_within_radius;
use crate::utils::jwt::Claims;
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct AvailableJourneyResponse {
    pub id: Uuid,
    pub origin_city: CityInfo,
    pub destination_city: CityInfo,
    pub departure_time: DateTime<Utc>,
    pub available_seats: i32,
    pub has_driver: bool,
}

#[derive(Debug, Serialize)]
pub struct CityInfo {
    pub id: i32,
    pub name: String,
    pub center_lat: f64,
    pub center_lng: f64,
    pub pickup_radius_km: f64,
}

/// List available journeys for booking
pub async fn list_journeys(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<AvailableJourneyResponse>>> {
    let now = Utc::now();
    let journeys = journey::Entity::find().all(&state.db).await?;
    let cities = city::Entity::find().all(&state.db).await?;

    let mut responses = Vec::new();
    for j in journeys {
        // Skip past journeys
        if j.departure_time.with_timezone(&Utc) < now {
            continue;
        }

        let origin = cities.iter().find(|c| c.id == j.origin_city_id);
        let dest = cities.iter().find(|c| c.id == j.destination_city_id);

        if origin.is_none() || dest.is_none() {
            continue;
        }

        let origin = origin.unwrap();
        let dest = dest.unwrap();

        let booked: i32 = booking::Entity::find()
            .filter(booking::Column::JourneyId.eq(j.id))
            .all(&state.db)
            .await?
            .iter()
            .map(|b| b.seats)
            .sum();

        let available = j.total_seats - booked;

        responses.push(AvailableJourneyResponse {
            id: j.id,
            origin_city: CityInfo {
                id: origin.id,
                name: origin.name.clone(),
                center_lat: origin.center_lat,
                center_lng: origin.center_lng,
                pickup_radius_km: origin.pickup_radius_km,
            },
            destination_city: CityInfo {
                id: dest.id,
                name: dest.name.clone(),
                center_lat: dest.center_lat,
                center_lng: dest.center_lng,
                pickup_radius_km: dest.pickup_radius_km,
            },
            departure_time: j.departure_time.with_timezone(&Utc),
            available_seats: available,
            has_driver: j.driver_id.is_some(),
        });
    }

    Ok(Json(responses))
}

/// Get journey details
pub async fn get_journey(
    State(state): State<AppState>,
    Path(journey_id): Path<Uuid>,
) -> AppResult<Json<AvailableJourneyResponse>> {
    let journey = journey::Entity::find_by_id(journey_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Perjalanan tidak ditemukan".to_string()))?;

    let cities = city::Entity::find().all(&state.db).await?;
    let origin = cities
        .iter()
        .find(|c| c.id == journey.origin_city_id)
        .ok_or_else(|| AppError::Internal("Origin city not found".to_string()))?;
    let dest = cities
        .iter()
        .find(|c| c.id == journey.destination_city_id)
        .ok_or_else(|| AppError::Internal("Destination city not found".to_string()))?;

    let booked: i32 = booking::Entity::find()
        .filter(booking::Column::JourneyId.eq(journey.id))
        .all(&state.db)
        .await?
        .iter()
        .map(|b| b.seats)
        .sum();

    Ok(Json(AvailableJourneyResponse {
        id: journey.id,
        origin_city: CityInfo {
            id: origin.id,
            name: origin.name.clone(),
            center_lat: origin.center_lat,
            center_lng: origin.center_lng,
            pickup_radius_km: origin.pickup_radius_km,
        },
        destination_city: CityInfo {
            id: dest.id,
            name: dest.name.clone(),
            center_lat: dest.center_lat,
            center_lng: dest.center_lng,
            pickup_radius_km: dest.pickup_radius_km,
        },
        departure_time: journey.departure_time.with_timezone(&Utc),
        available_seats: journey.total_seats - booked,
        has_driver: journey.driver_id.is_some(),
    }))
}

// ============ Booking Management ============

#[derive(Debug, Deserialize)]
pub struct CreateBookingRequest {
    pub journey_id: Uuid,
    pub seats: i32,
    pub pickup_lat: f64,
    pub pickup_lng: f64,
}

#[derive(Debug, Serialize)]
pub struct BookingResponse {
    pub id: Uuid,
    pub journey_id: Uuid,
    pub origin_city: String,
    pub destination_city: String,
    pub departure_time: DateTime<Utc>,
    pub seats: i32,
    pub pickup_lat: f64,
    pub pickup_lng: f64,
    pub created_at: DateTime<Utc>,
}

/// Create a booking
pub async fn create_booking(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<CreateBookingRequest>,
) -> AppResult<StatusCode> {
    // Validate journey
    let journey = journey::Entity::find_by_id(payload.journey_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Perjalanan tidak ditemukan".to_string()))?;

    // Check journey is in the future
    if journey.departure_time.with_timezone(&Utc) < Utc::now() {
        return Err(AppError::BadRequest("Perjalanan yang sudah lewat tidak dapat dipesan".to_string()));
    }

    // Check seat availability
    let booked: i32 = booking::Entity::find()
        .filter(booking::Column::JourneyId.eq(journey.id))
        .all(&state.db)
        .await?
        .iter()
        .map(|b| b.seats)
        .sum();

    let available = journey.total_seats - booked;
    if payload.seats > available {
        return Err(AppError::BadRequest(format!(
            "Hanya tersisa {} kursi",
            available
        )));
    }

    if payload.seats <= 0 {
        return Err(AppError::BadRequest(
            "Minimal memesan 1 kursi".to_string(),
        ));
    }

    // Validate pickup point is within origin city radius
    let origin_city = city::Entity::find_by_id(journey.origin_city_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::Internal("Origin city not found".to_string()))?;

    if !is_within_radius(
        payload.pickup_lat,
        payload.pickup_lng,
        origin_city.center_lat,
        origin_city.center_lng,
        origin_city.pickup_radius_km,
    ) {
        return Err(AppError::BadRequest(format!(
            "Titik jemput harus dalam radius {} km dari pusat kota {}",
            origin_city.pickup_radius_km, origin_city.name
        )));
    }

    // Check if user already has a booking for this journey
    let existing = booking::Entity::find()
        .filter(booking::Column::JourneyId.eq(journey.id))
        .filter(booking::Column::UserId.eq(claims.sub))
        .one(&state.db)
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "Anda sudah memesan tiket untuk perjalanan ini".to_string(),
        ));
    }

    // Create booking
    let booking_id = Uuid::new_v4();
    let new_booking = booking::ActiveModel {
        id: Set(booking_id),
        journey_id: Set(journey.id),
        user_id: Set(claims.sub),
        seats: Set(payload.seats),
        pickup_lat: Set(payload.pickup_lat),
        pickup_lng: Set(payload.pickup_lng),
        ..Default::default()
    };

    new_booking.insert(&state.db).await?;

    Ok(StatusCode::CREATED)
}

/// List user's bookings
pub async fn my_bookings(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> AppResult<Json<Vec<BookingResponse>>> {
    let bookings = booking::Entity::find()
        .filter(booking::Column::UserId.eq(claims.sub))
        .all(&state.db)
        .await?;

    let journeys = journey::Entity::find().all(&state.db).await?;
    let cities = city::Entity::find().all(&state.db).await?;

    let responses: Vec<BookingResponse> = bookings
        .into_iter()
        .filter_map(|b| {
            let journey = journeys.iter().find(|j| j.id == b.journey_id)?;
            let origin = cities.iter().find(|c| c.id == journey.origin_city_id);
            let dest = cities.iter().find(|c| c.id == journey.destination_city_id);

            Some(BookingResponse {
                id: b.id,
                journey_id: journey.id,
                origin_city: origin.map(|c| c.name.clone()).unwrap_or_default(),
                destination_city: dest.map(|c| c.name.clone()).unwrap_or_default(),
                departure_time: journey.departure_time.with_timezone(&Utc),
                seats: b.seats,
                pickup_lat: b.pickup_lat,
                pickup_lng: b.pickup_lng,
                created_at: b.created_at.with_timezone(&Utc),
            })
        })
        .collect();

    Ok(Json(responses))
}

/// Cancel a booking
pub async fn cancel_booking(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(booking_id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let booking = booking::Entity::find_by_id(booking_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Pesanan tidak ditemukan".to_string()))?;

    // Verify ownership
    if booking.user_id != claims.sub {
        return Err(AppError::Forbidden(
            "Anda hanya dapat membatalkan pesanan Anda sendiri".to_string(),
        ));
    }

    // Check if journey is still in the future
    let journey = journey::Entity::find_by_id(booking.journey_id)
        .one(&state.db)
        .await?;

    if let Some(j) = journey {
        if j.departure_time.with_timezone(&Utc) < Utc::now() {
            return Err(AppError::BadRequest(
                "Pesanan untuk perjalanan yang sudah lewat tidak dapat dibatalkan".to_string(),
            ));
        }
    }

    booking::Entity::delete_by_id(booking_id)
        .exec(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
