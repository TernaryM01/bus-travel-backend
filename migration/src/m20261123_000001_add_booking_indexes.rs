use sea_orm_migration::prelude::*;

/// Indexes for the queries that look up bookings by journey or by user. Besides
/// speed, they let Postgres track what a SERIALIZABLE transaction reads row by row
/// instead of for the whole table, so concurrent requests about different journeys
/// or users don't look like conflicts (see src/db/request_txn.rs).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_booking_journey_id")
                    .table(Booking::Table)
                    .col(Booking::JourneyId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_booking_user_id")
                    .table(Booking::Table)
                    .col(Booking::UserId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(Index::drop().name("idx_booking_user_id").to_owned())
            .await?;

        manager
            .drop_index(Index::drop().name("idx_booking_journey_id").to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Booking {
    Table,
    JourneyId,
    UserId,
}
