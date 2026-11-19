use axum::{extract::State, http::StatusCode, Extension};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::extract::{Json, Path};
use crate::entities::booking::PickupStatus;
use crate::entities::journey::JourneyStatus;
use crate::entities::{booking, city, journey};
use crate::error::{AppError, AppResult};
use crate::handlers::shared::{self, JourneyPassengersResponse};
use crate::handlers::traveller::CityInfo;
use crate::utils::jwt::Claims;
use crate::AppState;

#[derive(Debug, Serialize)]
pub struct DriverJourneyResponse {
    pub id: Uuid,
    pub origin_city: CityInfo,
    pub destination_city: CityInfo,
    pub departure_time: DateTime<Utc>,
    pub total_seats: i32,
    pub booked_seats: i32,
    pub status: JourneyStatus,
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

        responses.push(DriverJourneyResponse {
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
            total_seats: j.total_seats,
            booked_seats: booked,
            status: j.status,
        });
    }

    Ok(Json(responses))
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
        .ok_or_else(|| AppError::NotFound("Perjalanan tidak ditemukan".to_string()))?;

    if journey.driver_id != Some(claims.sub) {
        return Err(AppError::Forbidden(
            "Anda tidak ditugaskan pada perjalanan ini".to_string(),
        ));
    }

    Ok(Json(shared::journey_passengers(&state.db, journey).await?))
}

// ============ Pickup Process ============

#[derive(Debug, Deserialize)]
pub struct UpdateJourneyStatusRequest {
    pub status: JourneyStatus,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePickupStatusRequest {
    pub pickup_status: PickupStatus,
}

/// Position of a status in the journey's sequence of stages
fn stage_index(status: JourneyStatus) -> i32 {
    match status {
        JourneyStatus::Scheduled => 0,
        JourneyStatus::PickingUp => 1,
        JourneyStatus::InTransit => 2,
        JourneyStatus::Finished => 3,
    }
}

/// Move a journey one stage forwards or backwards
pub async fn update_journey_status(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(journey_id): Path<Uuid>,
    Json(payload): Json<UpdateJourneyStatusRequest>,
) -> AppResult<StatusCode> {
    // Verify the journey is assigned to this driver
    let journey = journey::Entity::find_by_id(journey_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Perjalanan tidak ditemukan".to_string()))?;

    if journey.driver_id != Some(claims.sub) {
        return Err(AppError::Forbidden(
            "Anda tidak ditugaskan pada perjalanan ini".to_string(),
        ));
    }

    if (stage_index(payload.status) - stage_index(journey.status)).abs() != 1 {
        return Err(AppError::Conflict(
            "Status perjalanan hanya dapat diubah satu tahap maju atau mundur".to_string(),
        ));
    }

    // Every passenger must be either picked up or marked as no-show before leaving
    if payload.status == JourneyStatus::InTransit {
        let pending = booking::Entity::find()
            .filter(booking::Column::JourneyId.eq(journey.id))
            .filter(booking::Column::PickupStatus.eq(PickupStatus::Pending))
            .count(&state.db)
            .await?;

        if pending > 0 {
            return Err(AppError::Conflict(format!(
                "Masih ada {} penumpang yang belum dijemput. Tandai sebagai sudah dijemput atau tidak hadir terlebih dahulu",
                pending
            )));
        }
    }

    let mut active: journey::ActiveModel = journey.into();
    active.status = Set(payload.status);
    active.update(&state.db).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Set a passenger's pickup status while the journey is in the pickup stage
pub async fn update_pickup_status(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((journey_id, booking_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<UpdatePickupStatusRequest>,
) -> AppResult<StatusCode> {
    // Verify the journey is assigned to this driver
    let journey = journey::Entity::find_by_id(journey_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Perjalanan tidak ditemukan".to_string()))?;

    if journey.driver_id != Some(claims.sub) {
        return Err(AppError::Forbidden(
            "Anda tidak ditugaskan pada perjalanan ini".to_string(),
        ));
    }

    // The booking must belong to this journey
    let booking_record = booking::Entity::find_by_id(booking_id)
        .filter(booking::Column::JourneyId.eq(journey.id))
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Pesanan tidak ditemukan".to_string()))?;

    if journey.status != JourneyStatus::PickingUp {
        return Err(AppError::Conflict(
            "Status penjemputan hanya dapat diubah saat perjalanan dalam tahap penjemputan".to_string(),
        ));
    }

    let mut active: booking::ActiveModel = booking_record.into();
    active.pickup_status = Set(payload.pickup_status);
    active.update(&state.db).await?;

    Ok(StatusCode::NO_CONTENT)
}
