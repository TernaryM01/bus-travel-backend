use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(City::Table)
                    .if_not_exists()
                    .col(pk_auto(City::Id))
                    .col(string_len(City::Name, 50).not_null().unique_key())
                    .col(double(City::CenterLat).not_null())
                    .col(double(City::CenterLng).not_null())
                    .col(double(City::PickupRadiusKm).not_null())
                    .to_owned(),
            )
            .await?;

        // Seed cities
        let insert = Query::insert()
            .into_table(City::Table)
            .columns([City::Name, City::CenterLat, City::CenterLng, City::PickupRadiusKm])
            .values_panic(["Jakarta".into(), (-6.2088).into(), (106.8456).into(), (10.0).into()])
            .values_panic(["Bandung".into(), (-6.9175).into(), (107.6191).into(), (7.0).into()])
            .to_owned();

        manager.exec_stmt(insert).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(City::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum City {
    Table,
    Id,
    Name,
    CenterLat,
    CenterLng,
    PickupRadiusKm,
}
