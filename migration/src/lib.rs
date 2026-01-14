pub use sea_orm_migration::prelude::*;

mod m20231228_000001_create_cities;
mod m20231228_000002_create_users;
mod m20231228_000003_create_journeys;
mod m20231228_000004_create_bookings;
mod m20260114_000001_add_google_oauth;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20231228_000001_create_cities::Migration),
            Box::new(m20231228_000002_create_users::Migration),
            Box::new(m20231228_000003_create_journeys::Migration),
            Box::new(m20231228_000004_create_bookings::Migration),
            Box::new(m20260114_000001_add_google_oauth::Migration),
        ]
    }
}
