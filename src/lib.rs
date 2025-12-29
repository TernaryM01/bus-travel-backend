pub mod config;
pub mod db;
pub mod entities;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod routes;
pub mod utils;

use sea_orm::DatabaseConnection;

pub use config::Config;
pub use error::{AppError, AppResult};

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub config: Config,
}
