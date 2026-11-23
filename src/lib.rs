pub mod config;
pub mod db;
pub mod entities;
pub mod error;
pub mod extract;
pub mod handlers;
pub mod middleware;
pub mod midtrans;
pub mod routes;
pub mod utils;

use db::CachedDb;
use midtrans::Midtrans;

pub use config::Config;
pub use error::{AppError, AppResult};

#[derive(Clone)]
pub struct AppState {
    pub db: CachedDb,
    pub config: Config,
    pub midtrans: Midtrans,
}
