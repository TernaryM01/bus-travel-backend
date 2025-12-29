use sea_orm::{Database, DatabaseConnection};

use crate::config::Config;
use crate::error::{AppError, AppResult};

pub async fn connect(config: &Config) -> AppResult<DatabaseConnection> {
    Database::connect(&config.database_url)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to connect to database: {}", e)))
}
