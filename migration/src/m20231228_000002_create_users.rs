use sea_orm_migration::{prelude::*, schema::*, sea_orm::sea_query::extension::postgres::Type};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create user role enum
        manager
            .create_type(
                Type::create()
                    .as_enum(UserRole::Enum)
                    .values([UserRole::Admin, UserRole::Driver, UserRole::Traveller])
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(User::Table)
                    .if_not_exists()
                    .col(uuid(User::Id).primary_key())
                    .col(string_len(User::Email, 255).not_null().unique_key())
                    .col(string_len(User::PasswordHash, 255).not_null())
                    .col(string_len(User::Name, 100).not_null())
                    .col(
                        ColumnDef::new(User::Role)
                            .custom(UserRole::Enum)
                            .not_null(),
                    )
                    .col(
                        timestamp_with_time_zone(User::CreatedAt)
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(User::Table).to_owned())
            .await?;

        manager
            .drop_type(Type::drop().name(UserRole::Enum).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum User {
    Table,
    Id,
    Email,
    PasswordHash,
    Name,
    Role,
    CreatedAt,
}

#[derive(DeriveIden)]
pub enum UserRole {
    #[sea_orm(iden = "user_role")]
    Enum,
    #[sea_orm(iden = "admin")]
    Admin,
    #[sea_orm(iden = "driver")]
    Driver,
    #[sea_orm(iden = "traveller")]
    Traveller,
}
