# Bus Travel Backend

A Rust backend for a bus travel booking system between Jakarta and Bandung, built with **Axum** and **SeaORM**.

## Features

- **Role-based Access Control**: Admin, Driver, and Traveller roles with JWT authentication
- **Journey Management**: Admins create bus journeys between Jakarta ↔ Bandung
- **Booking System**: Travellers book seats and specify pickup points within allowed radius
- **Driver Dashboard**: Drivers view assigned journeys and passenger pickup locations
- **Rate Limiting**: 100 requests per 60 seconds per IP address

## Tech Stack

| Component | Technology |
|-----------|------------|
| Framework | Axum 0.7 |
| ORM | SeaORM 1.0 |
| Database | PostgreSQL |
| Auth | JWT (jsonwebtoken) + Argon2 password hashing |
| Rate Limiting | tower-governor |

## Project Structure

```
backend/
├── Cargo.toml              # Main dependencies + workspace config
├── .env                    # Environment configuration
├── migration/              # Database migrations (separate crate)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       └── m20231228_*.rs  # Migration files
└── src/
    ├── main.rs             # Entry point, server setup
    ├── lib.rs              # AppState and re-exports
    ├── config.rs           # Environment configuration
    ├── error.rs            # Unified error handling
    ├── db/                 # Database connection
    ├── entities/           # SeaORM entity models
    ├── handlers/           # Route handlers by role
    ├── middleware/         # Auth & role middleware
    ├── routes/             # Route definitions
    └── utils/              # JWT and geolocation helpers
```

## Prerequisites

- Rust 1.70+
- PostgreSQL 14+

## Setup

### 1. Database

```bash
# Create the database
createdb bus_travel
```

### 2. Environment

Copy and configure `.env`:

```env
DATABASE_URL=postgres://postgres:password@localhost:5432/bus_travel
JWT_SECRET=your-secret-key-change-in-production
JWT_EXPIRATION_HOURS=24
SERVER_HOST=0.0.0.0
SERVER_PORT=3000
```

### 3. Run

```bash
# Development
cargo run

# Production
cargo run --release
```

The server will:
1. Connect to PostgreSQL
2. Run migrations automatically
3. Create admin account if not exists
4. Listen on the configured port

## Default Admin Account

On first run, an admin account is created:
- **Email**: `admin@bustravel.com`
- **Password**: `admin123`

> ⚠️ Change these credentials in production!

## API Documentation

See [API.md](./API.md) for complete API documentation.

## Cities

| City | Center Coordinates | Pickup Radius |
|------|-------------------|---------------|
| Jakarta | -6.2088, 106.8456 | 10 km |
| Bandung | -6.9175, 107.6191 | 7 km |

## License

MIT
