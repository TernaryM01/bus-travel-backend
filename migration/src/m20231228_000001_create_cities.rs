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
            .values_panic(["Kupang".into(), (-10.1836).into(), (123.6257).into(), (5.0).into()])
            .values_panic(["Soe".into(), (-9.8684).into(), (124.2861).into(), (2.0).into()])
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
