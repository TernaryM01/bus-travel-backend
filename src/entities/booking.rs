use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "pickup_status")]
pub enum PickupStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "picked_up")]
    PickedUp,
    #[sea_orm(string_value = "no_show")]
    NoShow,
}

/// Only Midtrans's answers move a booking out of `Pending`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "payment_status")]
pub enum PaymentStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "paid")]
    Paid,
    #[sea_orm(string_value = "expired")]
    Expired,
    /// Declined, cancelled or errored at Midtrans
    #[sea_orm(string_value = "rejected")]
    Rejected,
    /// Still pending when pickup started or the scheduled departure time passed; whether it was paid is unknown
    #[sea_orm(string_value = "failed")]
    Failed,
    /// Cancelled by the traveller before paying
    #[sea_orm(string_value = "cancelled")]
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "booking")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub journey_id: Uuid,
    pub user_id: Uuid,
    pub seats: i32,
    pub pickup_lat: f64,
    pub pickup_lng: f64,
    pub created_at: DateTimeWithTimeZone,
    pub pickup_status: PickupStatus,
    pub payment_status: PaymentStatus,
    /// Rupiah
    pub amount: i64,
    #[sea_orm(unique)]
    pub payment_order_id: Option<String>,
    /// Snap page token, to close the page
    pub payment_token: Option<String>,
    pub payment_url: Option<String>,
    pub payment_expires_at: Option<DateTimeWithTimeZone>,
    pub paid_at: Option<DateTimeWithTimeZone>,
    /// Midtrans payment_type
    pub payment_method: Option<String>,
}

impl Model {
    pub fn holds_seats(&self) -> bool {
        matches!(self.payment_status, PaymentStatus::Paid | PaymentStatus::Pending)
    }

    pub fn is_paid(&self) -> bool {
        self.payment_status == PaymentStatus::Paid
    }
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::journey::Entity",
        from = "Column::JourneyId",
        to = "super::journey::Column::Id"
    )]
    Journey,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
}

impl Related<super::journey::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Journey.def()
    }
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
