# Bus Travel Backend

A Rust backend for a bus travel booking system between Kupang and Soe, built with **Axum** and **SeaORM**.

## Features

- **Role-based Access Control**: Admin, Driver, and Traveller roles with JWT authentication
- **Journey Management**: Admins create bus journeys between Kupang ↔ Soe
- **Booking System**: Travellers book seats and specify pickup points within allowed radius
- **Driver Dashboard**: Drivers view assigned journeys and passenger pickup locations
- **Rate Limiting**: Per-IP and per-user limits (see [API.md](./API.md#rate-limiting))

## Tech Stack

| Component | Technology |
|-----------|------------|
| Framework | Axum 0.8 |
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
OAUTH_CLIENT_ID=your-google-oauth-client-id
```

`OAUTH_CLIENT_ID` is required: it's the Google OAuth client ID used to verify "Sign in with Google" tokens. The server won't start without it.

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

## Database Read Cache

All database access goes through `CachedDb` (`src/db/cached.rs`), a drop-in
replacement for SeaORM's `DatabaseConnection`. Repeated reads are answered from
memory; any write invalidates every cached result for the tables it touches,
including tables changed by foreign-key cascades. New queries need no extra code.
Cap its memory with `DB_CACHE_MAX_MB` (default 64).

It assumes:
- this backend is the only writer and runs as a single instance (no overlapping
  instances during redeploys, no manual edits with psql while it runs);
- `timestamptz` entity fields use `DateTimeWithTimeZone`.

## Concurrent Requests

Every write request (POST, PUT, DELETE) runs in one SERIALIZABLE database
transaction (`src/db/request_txn.rs`). When it conflicts with a concurrent
request (e.g. two travellers booking the last seat), Postgres aborts it and it is
run again automatically. Handlers need no code for this: keep using `state.db`.
GET requests get no transaction and are answered from the read cache.

Keep in mind:
- a write request may run more than once, so anything it does outside the
  database must be safe to repeat;
- an error response (4xx or 5xx) rolls back everything the request wrote;
- GET handlers must not write to the database.


## Default Admin Account

On startup, if no account with the admin role exists (e.g. on first run),
an admin account is created:
- **Email**: `admin@bustravel.com`
- **Password**: `admin123`

> ⚠️ Change these credentials in production!

## API Documentation

See [API.md](./API.md) for complete API documentation.

## Cities

| City | Center Coordinates | Pickup Radius |
|------|-------------------|---------------|
| Kupang | -10.1836, 123.6257 | 5 km |
| Soe | -9.8684, 124.2861 | 2 km |

## License

MIT
