use sea_orm_migration::{prelude::*, schema::*};

use super::m20231228_000001_create_cities::City;
use super::m20231228_000002_create_users::User;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Journey::Table)
                    .if_not_exists()
                    .col(uuid(Journey::Id).primary_key())
                    .col(integer(Journey::OriginCityId).not_null())
                    .col(integer(Journey::DestinationCityId).not_null())
                    .col(timestamp_with_time_zone(Journey::DepartureTime).not_null())
                    .col(integer(Journey::TotalSeats).not_null())
                    .col(uuid_null(Journey::DriverId))
                    .col(
                        timestamp_with_time_zone(Journey::CreatedAt)
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_journey_origin_city")
                            .from(Journey::Table, Journey::OriginCityId)
                            .to(City::Table, City::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_journey_destination_city")
                            .from(Journey::Table, Journey::DestinationCityId)
                            .to(City::Table, City::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_journey_driver")
                            .from(Journey::Table, Journey::DriverId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Journey::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum Journey {
    Table,
    Id,
    OriginCityId,
    DestinationCityId,
    DepartureTime,
    TotalSeats,
    DriverId,
    CreatedAt,
}
