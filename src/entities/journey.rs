use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "journey")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub origin_city_id: i32,
    pub destination_city_id: i32,
    pub departure_time: DateTimeWithTimeZone,
    pub total_seats: i32,
    pub driver_id: Option<Uuid>,
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::city::Entity",
        from = "Column::OriginCityId",
        to = "super::city::Column::Id"
    )]
    OriginCity,
    #[sea_orm(
        belongs_to = "super::city::Entity",
        from = "Column::DestinationCityId",
        to = "super::city::Column::Id"
    )]
    DestinationCity,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::DriverId",
        to = "super::user::Column::Id"
    )]
    Driver,
    #[sea_orm(has_many = "super::booking::Entity")]
    Bookings,
}

impl Related<super::booking::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Bookings.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
