use sea_orm_migration::{prelude::*, schema::*};

use super::m20231228_000002_create_users::User;
use super::m20231228_000003_create_journeys::Journey;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Booking::Table)
                    .if_not_exists()
                    .col(uuid(Booking::Id).primary_key())
                    .col(uuid(Booking::JourneyId).not_null())
                    .col(uuid(Booking::UserId).not_null())
                    .col(integer(Booking::Seats).not_null())
                    .col(double(Booking::PickupLat).not_null())
                    .col(double(Booking::PickupLng).not_null())
                    .col(
                        timestamp_with_time_zone(Booking::CreatedAt)
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_booking_journey")
                            .from(Booking::Table, Booking::JourneyId)
                            .to(Journey::Table, Journey::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_booking_user")
                            .from(Booking::Table, Booking::UserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Booking::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum Booking {
    Table,
    Id,
    JourneyId,
    UserId,
    Seats,
    PickupLat,
    PickupLng,
    CreatedAt,
}
