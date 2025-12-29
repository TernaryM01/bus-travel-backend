use axum::{
    extract::{Path, State},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use uuid::Uuid;

use crate::entities::{booking, city, journey};
use crate::error::{AppError, AppResult};
use crate::utils::jwt::Claims;
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct DriverJourneyResponse {
    pub id: Uuid,
    pub origin_city: String,
    pub destination_city: String,
    pub departure_time: DateTime<Utc>,
    pub total_seats: i32,
    pub booked_seats: i32,
}

/// List journeys assigned to the logged-in driver
pub async fn my_journeys(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> AppResult<Json<Vec<DriverJourneyResponse>>> {
    let journeys = journey::Entity::find()
        .filter(journey::Column::DriverId.eq(claims.sub))
        .all(&state.db)
        .await?;

    let cities = city::Entity::find().all(&state.db).await?;

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

        responses.push(DriverJourneyResponse {
            id: j.id,
            origin_city: origin.map(|c| c.name.clone()).unwrap_or_default(),
            destination_city: dest.map(|c| c.name.clone()).unwrap_or_default(),
            departure_time: j.departure_time.with_timezone(&Utc),
            total_seats: j.total_seats,
            booked_seats: booked,
        });
    }

    Ok(Json(responses))
}

#[derive(Debug, Serialize)]
pub struct PassengerPickupInfo {
    pub booking_id: Uuid,
    pub passenger_name: String,
    pub seats: i32,
    pub pickup_lat: f64,
    pub pickup_lng: f64,
}

#[derive(Debug, Serialize)]
pub struct JourneyPassengersResponse {
    pub journey_id: Uuid,
    pub origin_city: String,
    pub destination_city: String,
    pub departure_time: DateTime<Utc>,
    pub passengers: Vec<PassengerPickupInfo>,
}

/// Get passenger pickup points for a specific journey
pub async fn journey_passengers(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(journey_id): Path<Uuid>,
) -> AppResult<Json<JourneyPassengersResponse>> {
    // Verify the journey is assigned to this driver
    let journey = journey::Entity::find_by_id(journey_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Journey not found".to_string()))?;

    if journey.driver_id != Some(claims.sub) {
        return Err(AppError::Forbidden(
            "You are not assigned to this journey".to_string(),
        ));
    }

    let cities = city::Entity::find().all(&state.db).await?;
    let origin = cities.iter().find(|c| c.id == journey.origin_city_id);
    let dest = cities.iter().find(|c| c.id == journey.destination_city_id);

    // Get all bookings for this journey
    let bookings = booking::Entity::find()
        .filter(booking::Column::JourneyId.eq(journey_id))
        .all(&state.db)
        .await?;

    // Get user info for each booking
    use crate::entities::user;
    let users = user::Entity::find().all(&state.db).await?;

    let passengers: Vec<PassengerPickupInfo> = bookings
        .into_iter()
        .map(|b| {
            let user = users.iter().find(|u| u.id == b.user_id);
            PassengerPickupInfo {
                booking_id: b.id,
                passenger_name: user.map(|u| u.name.clone()).unwrap_or_default(),
                seats: b.seats,
                pickup_lat: b.pickup_lat,
                pickup_lng: b.pickup_lng,
            }
        })
        .collect();

    Ok(Json(JourneyPassengersResponse {
        journey_id: journey.id,
        origin_city: origin.map(|c| c.name.clone()).unwrap_or_default(),
        destination_city: dest.map(|c| c.name.clone()).unwrap_or_default(),
        departure_time: journey.departure_time.with_timezone(&Utc),
        passengers,
    }))
}
