use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add google_id column (nullable, unique)
        manager
            .alter_table(
                Table::alter()
                    .table(User::Table)
                    .add_column(string_len_null(User::GoogleId, 255).unique_key())
                    .to_owned(),
            )
            .await?;

        // Make password_hash nullable for Google-only users
        manager
            .alter_table(
                Table::alter()
                    .table(User::Table)
                    .modify_column(string_len_null(User::PasswordHash, 255))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Make password_hash NOT NULL again (may fail if Google-only users exist)
        manager
            .alter_table(
                Table::alter()
                    .table(User::Table)
                    .modify_column(string_len(User::PasswordHash, 255).not_null())
                    .to_owned(),
            )
            .await?;

        // Drop google_id column
        manager
            .alter_table(
                Table::alter()
                    .table(User::Table)
                    .drop_column(User::GoogleId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum User {
    Table,
    PasswordHash,
    GoogleId,
}
