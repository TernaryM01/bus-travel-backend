use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use uuid::Uuid;

use crate::db::CachedDb;
use crate::entities::booking::PickupStatus;
use crate::entities::journey::JourneyStatus;
use crate::entities::{booking, city, journey, user};
use crate::error::AppResult;

// ============ Journey Passengers (same view for the driver and the admin) ============

#[derive(Debug, Serialize)]
pub struct PassengerPickupInfo {
    pub booking_id: Uuid,
    pub passenger_name: String,
    pub seats: i32,
    pub pickup_lat: f64,
    pub pickup_lng: f64,
    pub pickup_status: PickupStatus,
}

#[derive(Debug, Serialize)]
pub struct JourneyPassengersResponse {
    pub journey_id: Uuid,
    pub origin_city: String,
    pub destination_city: String,
    pub departure_time: DateTime<Utc>,
    pub status: JourneyStatus,
    pub passengers: Vec<PassengerPickupInfo>,
}

/// Build the passenger list of a journey. Access checks are up to the caller.
pub async fn journey_passengers(
    db: &CachedDb,
    journey: journey::Model,
) -> AppResult<JourneyPassengersResponse> {
    let cities = city::Entity::find().all(db).await?;
    let origin = cities.iter().find(|c| c.id == journey.origin_city_id);
    let dest = cities.iter().find(|c| c.id == journey.destination_city_id);

    // Get all bookings for this journey
    let bookings = booking::Entity::find()
        .filter(booking::Column::JourneyId.eq(journey.id))
        .order_by_asc(booking::Column::CreatedAt)
        .order_by_asc(booking::Column::Id)
        .all(db)
        .await?;

    // Get user info for each booking
    let users = user::Entity::find().all(db).await?;

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
                pickup_status: b.pickup_status,
            }
        })
        .collect();

    Ok(JourneyPassengersResponse {
        journey_id: journey.id,
        origin_city: origin.map(|c| c.name.clone()).unwrap_or_default(),
        destination_city: dest.map(|c| c.name.clone()).unwrap_or_default(),
        departure_time: journey.departure_time.with_timezone(&Utc),
        status: journey.status,
        passengers,
    })
}
