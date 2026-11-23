use std::env;

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_expiration_hours: i64,
    pub server_host: String,
    pub server_port: u16,
    pub oauth_client_id: String,
    /// Memory cap for the database read cache, in megabytes.
    pub db_cache_max_mb: u64,
    pub midtrans_server_key: String,
    pub midtrans_is_production: bool,
    pub frontend_url: String,
    pub payment_hold_minutes: i64,
    /// Rupiah
    pub max_price_per_seat: i64,
}

impl Config {
    pub fn from_env() -> Self {
        dotenvy::dotenv().ok();

        Self {
            database_url: env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            jwt_secret: env::var("JWT_SECRET").expect("JWT_SECRET must be set"),
            jwt_expiration_hours: env::var("JWT_EXPIRATION_HOURS")
                .unwrap_or_else(|_| "24".to_string())
                .parse()
                .expect("JWT_EXPIRATION_HOURS must be a number"),
            server_host: env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            server_port: env::var("PORT")
                .or_else(|_| env::var("SERVER_PORT"))
                .unwrap_or_else(|_| "3000".to_string())
                .parse()
                .expect("Server port must be a number"),
            oauth_client_id: env::var("OAUTH_CLIENT_ID").expect("OAUTH_CLIENT_ID must be set"),
            db_cache_max_mb: env::var("DB_CACHE_MAX_MB")
                .unwrap_or_else(|_| "64".to_string())
                .parse()
                .expect("DB_CACHE_MAX_MB must be a number"),
            midtrans_server_key: env::var("MIDTRANS_SERVER_KEY")
                .expect("MIDTRANS_SERVER_KEY must be set"),
            midtrans_is_production: env::var("MIDTRANS_IS_PRODUCTION")
                .map(|v| v.trim().eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            frontend_url: env::var("FRONTEND_URL")
                .unwrap_or_else(|_| "http://localhost:5173".to_string())
                .trim_end_matches('/')
                .to_string(),
            payment_hold_minutes: env::var("PAYMENT_HOLD_MINUTES")
                .unwrap_or_else(|_| "30".to_string())
                .parse()
                .ok()
                .filter(|m| (10..=24 * 60).contains(m))
                .expect("PAYMENT_HOLD_MINUTES must be a number of minutes between 10 and 1440"),
            max_price_per_seat: env::var("MAX_PRICE_PER_SEAT")
                .expect("MAX_PRICE_PER_SEAT must be set")
                .parse()
                .expect("MAX_PRICE_PER_SEAT must be a number"),
        }
    }

    pub fn server_addr(&self) -> String {
        format!("{}:{}", self.server_host, self.server_port)
    }
}
