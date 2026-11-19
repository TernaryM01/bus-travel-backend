use sea_orm_migration::{prelude::*, schema::*, sea_orm::sea_query::extension::postgres::Type};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // --- City: drop-off station (the single destination point in each city) ---
        // Added as nullable, filled for the seeded cities, then made NOT NULL.
        manager
            .alter_table(
                Table::alter()
                    .table(City::Table)
                    .add_column(string_len_null(City::StationName, 100))
                    .add_column(double_null(City::StationLat))
                    .add_column(double_null(City::StationLng))
                    .to_owned(),
            )
            .await?;

        // PLACEHOLDERS: the city centre coordinates. Replace with the real
        // bus station name and coordinates for each city.
        for (city, name, lat, lng) in [
            ("Kupang", "Pusat Kota Kupang", -10.1836, 123.6257),
            ("Soe", "Pusat Kota Soe", -9.8684, 124.2861),
        ] {
            let update = Query::update()
                .table(City::Table)
                .value(City::StationName, name)
                .value(City::StationLat, lat)
                .value(City::StationLng, lng)
                .and_where(Expr::col(City::Name).eq(city))
                .to_owned();
            manager.exec_stmt(update).await?;
        }

        manager
            .alter_table(
                Table::alter()
                    .table(City::Table)
                    .modify_column(string_len(City::StationName, 100).not_null())
                    .modify_column(double(City::StationLat).not_null())
                    .modify_column(double(City::StationLng).not_null())
                    .to_owned(),
            )
            .await?;

        // --- Journey: status ---
        manager
            .create_type(
                Type::create()
                    .as_enum(JourneyStatus::Enum)
                    .values([
                        JourneyStatus::Scheduled,
                        JourneyStatus::PickingUp,
                        JourneyStatus::InTransit,
                        JourneyStatus::Finished,
                    ])
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Journey::Table)
                    .add_column(
                        ColumnDef::new(Journey::Status)
                            .custom(JourneyStatus::Enum)
                            .not_null()
                            .default("scheduled"),
                    )
                    .to_owned(),
            )
            .await?;

        // --- Booking: pickup status ---
        manager
            .create_type(
                Type::create()
                    .as_enum(PickupStatus::Enum)
                    .values([PickupStatus::Pending, PickupStatus::PickedUp, PickupStatus::NoShow])
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Booking::Table)
                    .add_column(
                        ColumnDef::new(Booking::PickupStatus)
                            .custom(PickupStatus::Enum)
                            .not_null()
                            .default("pending"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Booking::Table)
                    .drop_column(Booking::PickupStatus)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_type(Type::drop().name(PickupStatus::Enum).to_owned())
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Journey::Table)
                    .drop_column(Journey::Status)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_type(Type::drop().name(JourneyStatus::Enum).to_owned())
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(City::Table)
                    .drop_column(City::StationName)
                    .drop_column(City::StationLat)
                    .drop_column(City::StationLng)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum City {
    Table,
    Name,
    StationName,
    StationLat,
    StationLng,
}

#[derive(DeriveIden)]
enum Journey {
    Table,
    Status,
}

#[derive(DeriveIden)]
enum Booking {
    Table,
    PickupStatus,
}

#[derive(DeriveIden)]
enum JourneyStatus {
    #[sea_orm(iden = "journey_status")]
    Enum,
    #[sea_orm(iden = "scheduled")]
    Scheduled,
    #[sea_orm(iden = "picking_up")]
    PickingUp,
    #[sea_orm(iden = "in_transit")]
    InTransit,
    #[sea_orm(iden = "finished")]
    Finished,
}

#[derive(DeriveIden)]
enum PickupStatus {
    #[sea_orm(iden = "pickup_status")]
    Enum,
    #[sea_orm(iden = "pending")]
    Pending,
    #[sea_orm(iden = "picked_up")]
    PickedUp,
    #[sea_orm(iden = "no_show")]
    NoShow,
}
