use sea_orm_migration::{prelude::*, schema::*, sea_orm::sea_query::extension::postgres::Type};

/// Ticket prices and Midtrans payments. Existing journeys become free (price 0)
/// and existing bookings paid.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Journey::Table)
                    .add_column(big_integer(Journey::PricePerSeat).not_null().default(0))
                    .to_owned(),
            )
            .await?;

        manager
            .create_type(
                Type::create()
                    .as_enum(PaymentStatus::Enum)
                    .values([
                        PaymentStatus::Pending,
                        PaymentStatus::Paid,
                        PaymentStatus::Expired,
                        PaymentStatus::Rejected,
                        PaymentStatus::Failed,
                        PaymentStatus::Cancelled,
                    ])
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Booking::Table)
                    .add_column(
                        ColumnDef::new(Booking::PaymentStatus)
                            .custom(PaymentStatus::Enum)
                            .not_null()
                            .default("paid"),
                    )
                    .add_column(big_integer(Booking::Amount).not_null().default(0))
                    .add_column(string_len_null(Booking::PaymentOrderId, 50))
                    .add_column(string_len_null(Booking::PaymentToken, 100))
                    .add_column(text_null(Booking::PaymentUrl))
                    .add_column(timestamp_with_time_zone_null(Booking::PaymentExpiresAt))
                    .add_column(timestamp_with_time_zone_null(Booking::PaidAt))
                    .add_column(string_len_null(Booking::PaymentMethod, 50))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_booking_payment_order_id")
                    .table(Booking::Table)
                    .col(Booking::PaymentOrderId)
                    .unique()
                    .if_not_exists()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(Index::drop().name("idx_booking_payment_order_id").to_owned())
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Booking::Table)
                    .drop_column(Booking::PaymentStatus)
                    .drop_column(Booking::Amount)
                    .drop_column(Booking::PaymentOrderId)
                    .drop_column(Booking::PaymentToken)
                    .drop_column(Booking::PaymentUrl)
                    .drop_column(Booking::PaymentExpiresAt)
                    .drop_column(Booking::PaidAt)
                    .drop_column(Booking::PaymentMethod)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_type(Type::drop().name(PaymentStatus::Enum).to_owned())
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Journey::Table)
                    .drop_column(Journey::PricePerSeat)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Journey {
    Table,
    PricePerSeat,
}

#[derive(DeriveIden)]
enum Booking {
    Table,
    PaymentStatus,
    Amount,
    PaymentOrderId,
    PaymentToken,
    PaymentUrl,
    PaymentExpiresAt,
    PaidAt,
    PaymentMethod,
}

#[derive(DeriveIden)]
enum PaymentStatus {
    #[sea_orm(iden = "payment_status")]
    Enum,
    #[sea_orm(iden = "pending")]
    Pending,
    #[sea_orm(iden = "paid")]
    Paid,
    #[sea_orm(iden = "expired")]
    Expired,
    #[sea_orm(iden = "rejected")]
    Rejected,
    #[sea_orm(iden = "failed")]
    Failed,
    #[sea_orm(iden = "cancelled")]
    Cancelled,
}
