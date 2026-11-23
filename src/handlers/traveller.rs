use axum::{extract::State, http::StatusCode, Extension};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::extract::{Json, Path};
use crate::entities::booking::{PaymentStatus, PickupStatus};
use crate::entities::journey::JourneyStatus;
use crate::entities::{booking, city, journey, user};
use crate::error::{AppError, AppResult};
use crate::handlers::payment::{self, PaymentResponse};
use crate::handlers::shared;
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
    pub price_per_seat: i64,
}

#[derive(Debug, Serialize)]
pub struct CityInfo {
    pub id: i32,
    pub name: String,
    pub center_lat: f64,
    pub center_lng: f64,
    pub pickup_radius_km: f64,
    pub station_name: String,
    pub station_lat: f64,
    pub station_lng: f64,
}

/// List available journeys for booking
pub async fn list_journeys(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<AvailableJourneyResponse>>> {
    let now = Utc::now();
    // Filtered here rather than in SQL: a query containing the current time would
    // be different on every request, so it could never be answered from the cache.
    let journeys: Vec<journey::Model> = journey::Entity::find()
        .order_by_desc(journey::Column::DepartureTime)
        .order_by_asc(journey::Column::Id)
        .all(&state.db)
        .await?
        .into_iter()
        // Skip past journeys, and journeys whose pickup has started
        // (they no longer take bookings)
        .filter(|j| j.departure_time.with_timezone(&Utc) >= now)
        .filter(|j| j.status == JourneyStatus::Scheduled)
        .collect();
    let cities = city::Entity::find().all(&state.db).await?;
    let journey_ids: Vec<Uuid> = journeys.iter().map(|j| j.id).collect();
    let booked_seats = shared::booked_seats(&state.db, &journey_ids).await?;

    let mut responses = Vec::new();
    for j in journeys {
        let origin = cities.iter().find(|c| c.id == j.origin_city_id);
        let dest = cities.iter().find(|c| c.id == j.destination_city_id);

        if origin.is_none() || dest.is_none() {
            continue;
        }

        let origin = origin.unwrap();
        let dest = dest.unwrap();

        let booked = booked_seats.get(&j.id).copied().unwrap_or(0);

        let available = j.total_seats - booked;

        responses.push(AvailableJourneyResponse {
            id: j.id,
            origin_city: CityInfo {
                id: origin.id,
                name: origin.name.clone(),
                center_lat: origin.center_lat,
                center_lng: origin.center_lng,
                pickup_radius_km: origin.pickup_radius_km,
                station_name: origin.station_name.clone(),
                station_lat: origin.station_lat,
                station_lng: origin.station_lng,
            },
            destination_city: CityInfo {
                id: dest.id,
                name: dest.name.clone(),
                center_lat: dest.center_lat,
                center_lng: dest.center_lng,
                pickup_radius_km: dest.pickup_radius_km,
                station_name: dest.station_name.clone(),
                station_lat: dest.station_lat,
                station_lng: dest.station_lng,
            },
            departure_time: j.departure_time.with_timezone(&Utc),
            available_seats: available,
            has_driver: j.driver_id.is_some(),
            price_per_seat: j.price_per_seat,
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

    let booked = shared::seats_taken(&state.db, journey.id, None).await?;

    Ok(Json(AvailableJourneyResponse {
        id: journey.id,
        origin_city: CityInfo {
            id: origin.id,
            name: origin.name.clone(),
            center_lat: origin.center_lat,
            center_lng: origin.center_lng,
            pickup_radius_km: origin.pickup_radius_km,
            station_name: origin.station_name.clone(),
            station_lat: origin.station_lat,
            station_lng: origin.station_lng,
        },
        destination_city: CityInfo {
            id: dest.id,
            name: dest.name.clone(),
            center_lat: dest.center_lat,
            center_lng: dest.center_lng,
            pickup_radius_km: dest.pickup_radius_km,
            station_name: dest.station_name.clone(),
            station_lat: dest.station_lat,
            station_lng: dest.station_lng,
        },
        departure_time: journey.departure_time.with_timezone(&Utc),
        available_seats: journey.total_seats - booked,
        has_driver: journey.driver_id.is_some(),
        price_per_seat: journey.price_per_seat,
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
    pub journey_status: JourneyStatus,
    pub seats: i32,
    pub pickup_lat: f64,
    pub pickup_lng: f64,
    pub created_at: DateTime<Utc>,
    pub payment_status: PaymentStatus,
    pub amount: i64,
    pub payment_expires_at: Option<DateTime<Utc>>,
    pub paid_at: Option<DateTime<Utc>>,
    pub payment_method: Option<String>,
}

/// Create a booking
pub async fn create_booking(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<CreateBookingRequest>,
) -> AppResult<(StatusCode, Json<PaymentResponse>)> {
    let now = Utc::now();

    // Validate journey
    let journey = journey::Entity::find_by_id(payload.journey_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Perjalanan tidak ditemukan".to_string()))?;

    // Check journey is in the future
    if journey.departure_time.with_timezone(&Utc) < now {
        return Err(AppError::BadRequest("Perjalanan yang sudah lewat tidak dapat dipesan".to_string()));
    }

    // Bookings close once the driver starts picking up passengers
    if journey.status != JourneyStatus::Scheduled {
        return Err(AppError::Conflict(
            "Perjalanan ini sudah tidak menerima pemesanan".to_string(),
        ));
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

    // An expired or failed booking for this journey is replaced
    let existing = booking::Entity::find()
        .filter(booking::Column::JourneyId.eq(journey.id))
        .filter(booking::Column::UserId.eq(claims.sub))
        .one(&state.db)
        .await?;
    if let Some(b) = &existing {
        match b.payment_status {
            PaymentStatus::Paid => {
                return Err(AppError::Conflict(
                    "Anda sudah memesan tiket untuk perjalanan ini".to_string(),
                ));
            }
            PaymentStatus::Pending => {
                return Err(AppError::Conflict(
                    "Anda sudah memesan kursi di perjalanan ini. Selesaikan pembayarannya di Tiket Saya".to_string(),
                ));
            }
            PaymentStatus::Expired | PaymentStatus::Rejected | PaymentStatus::Failed | PaymentStatus::Cancelled => {}
        }
    }

    // Check seat availability
    let taken = shared::seats_taken(&state.db, journey.id, existing.as_ref().map(|b| b.id)).await?;
    let available = journey.total_seats - taken;
    if payload.seats > available {
        return Err(AppError::BadRequest(format!(
            "Hanya tersisa {} kursi",
            available.max(0)
        )));
    }

    let amount = payment::total_price(&journey, payload.seats)?;
    let booking_id = existing.as_ref().map(|b| b.id).unwrap_or_else(Uuid::new_v4);

    let mut active = booking::ActiveModel {
        id: Set(booking_id),
        journey_id: Set(journey.id),
        user_id: Set(claims.sub),
        seats: Set(payload.seats),
        pickup_lat: Set(payload.pickup_lat),
        pickup_lng: Set(payload.pickup_lng),
        created_at: Set(now.into()),
        pickup_status: Set(PickupStatus::Pending),
        payment_status: Set(PaymentStatus::Paid),
        amount: Set(0),
        payment_order_id: Set(None),
        payment_token: Set(None),
        payment_url: Set(None),
        payment_expires_at: Set(None),
        paid_at: Set(Some(now.into())),
        payment_method: Set(None),
    };

    if amount > 0 {
        let traveller = user::Entity::find_by_id(claims.sub)
            .one(&state.db)
            .await?
            .ok_or_else(|| AppError::Unauthorized("Akun tidak ditemukan".to_string()))?;
        payment::new_payment_attempt(&state, booking_id, &journey, payload.seats, &traveller)
            .await?
            .apply_to(&state, &mut active, booking_id, amount);
    }

    let saved = if existing.is_some() {
        active.update(&state.db).await?
    } else {
        active.insert(&state.db).await?
    };

    Ok((StatusCode::CREATED, Json(PaymentResponse::of(&saved))))
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

    let mut responses: Vec<BookingResponse> = bookings
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
                journey_status: journey.status,
                seats: b.seats,
                pickup_lat: b.pickup_lat,
                pickup_lng: b.pickup_lng,
                created_at: b.created_at.with_timezone(&Utc),
                payment_status: b.payment_status,
                amount: b.amount,
                payment_expires_at: (b.payment_status == PaymentStatus::Pending)
                    .then_some(b.payment_expires_at)
                    .flatten()
                    .map(|t| t.with_timezone(&Utc)),
                paid_at: b.paid_at.map(|t| t.with_timezone(&Utc)),
                payment_method: b.payment_method.clone(),
            })
        })
        .collect();

    // By journey departure time, latest first
    responses.sort_by(|a, b| b.departure_time.cmp(&a.departure_time).then(a.id.cmp(&b.id)));

    Ok(Json(responses))
}
