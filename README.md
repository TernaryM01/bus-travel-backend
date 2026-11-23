# Bus Travel Backend

A Rust backend for a bus travel booking system between Kupang and Soe, built with **Axum** and **SeaORM**.

## Features

- **Role-based Access Control**: Admin, Driver, and Traveller roles with JWT authentication
- **Journey Management**: Admins create bus journeys between Kupang ↔ Soe
- **Booking System**: Travellers book seats and specify pickup points within allowed radius
- **Payments**: Travellers pay in IDR through Midtrans (bank transfer / virtual account, QRIS, e-wallets, cards)
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
    ├── midtrans.rs         # Midtrans payment gateway client
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
MIDTRANS_SERVER_KEY=SB-Mid-server-xxxxxxxxxxxxxxxx
FRONTEND_URL=http://localhost:5173
MAX_PRICE_PER_SEAT=1000000
```

`OAUTH_CLIENT_ID` is required: it's the Google OAuth client ID used to verify "Sign in with Google" tokens. The server won't start without it.

`MIDTRANS_SERVER_KEY` and `MAX_PRICE_PER_SEAT` are required too; see [Payments](#payments-midtrans).

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


## Payments (Midtrans)

Travellers pay through [Midtrans Snap](https://docs.midtrans.com/docs/snap-snap-integration-guide),
Midtrans's hosted payment page. IDR only. The code is in `src/midtrans.rs` (talking to Midtrans)
and `src/handlers/payment.rs` (what payments do to bookings).

### How it works

1. An admin sets a **price per seat** on each journey (0 = free).
2. A traveller books: the booking is created as `Pending`, **holds its seats** for
   `PAYMENT_HOLD_MINUTES` (default 30), and the traveller is sent to the Midtrans payment page.
3. Midtrans confirms the payment by calling
   `POST /api/payments/midtrans/notification` (signed with your server key). The backend also
   asks Midtrans directly when the traveller comes back from the payment page, or presses
   "Cek status" in Tiket Saya.
4. Paid bookings are confirmed and appear on the driver's passenger list.
5. At the deadline, or when the traveller cancels an unpaid booking ("Batalkan"), the backend
   tells Midtrans to stop the payment. Midtrans either confirms it's stopped (no money can arrive
   anymore: the seats are released at once) or says it's already paid (the booking is confirmed).
   Seats are never released without that confirmation; if Midtrans can't be reached, the backend
   keeps asking every 30 seconds and the seats stay held.
6. The deadline timers live in memory and are rebuilt from the database at startup, so a restart
   loses nothing. Midtrans enforces the deadline on its side too, even while the backend is down.

Paid bookings can't be cancelled by travellers. Admins can delete them; refunds are not automatic
(refund in the Midtrans dashboard, using the order id shown in the admin's booking list).

### Setup

1. Create a Midtrans account, switch the dashboard to the **Sandbox** environment, and copy the
   **Server Key** (`SB-Mid-server-...`) from *Settings > Access Keys* into `MIDTRANS_SERVER_KEY`.
2. Set `FRONTEND_URL` to the frontend's address: Midtrans sends travellers back to
   `<FRONTEND_URL>/my-bookings` after paying.
3. In the dashboard, *Settings > Configuration*, set **Payment Notification URL** to
   `https://<your-backend>/api/payments/midtrans/notification`.

Midtrans can't send notifications to `localhost`. During local development, payments are
confirmed when the traveller comes back to Tiket Saya, presses "Cek status", or at the deadline.

| Variable | Default | |
|---|---|---|
| `MIDTRANS_SERVER_KEY` | (required) | Sandbox keys start with `SB-Mid-server-` |
| `MIDTRANS_IS_PRODUCTION` | `false` | `true` = real money |
| `FRONTEND_URL` | `http://localhost:5173` | Where travellers return after paying |
| `PAYMENT_HOLD_MINUTES` | `30` | Time to pay (10 to 1440) |
| `MAX_PRICE_PER_SEAT` | (required) | Highest price per seat an admin can set, in rupiah |

### Testing without real money

With a sandbox key, nothing is charged. Book a seat, then on the Midtrans page either:
- choose **BCA Virtual Account**, copy the VA number, and pay it in the
  [BCA VA simulator](https://simulator.sandbox.midtrans.com/bca/va/index); or
- choose **QRIS**, copy the QR image's URL, and pay it in the
  [QRIS simulator](https://simulator.sandbox.midtrans.com/v2/qris/index); or
- pay by **card** with `4811 1111 1111 1114`, CVV `123`, any future expiry date, OTP `112233`.

Never pay a sandbox transaction with a real bank account, card or e-wallet.

More test methods (QRIS, GoPay, failing cards): [Testing Payment on Sandbox](https://docs.midtrans.com/docs/testing-payment-on-sandbox).
To test an expired payment, leave the page unpaid until the deadline.

### Going live

Use the Production server key, set `MIDTRANS_IS_PRODUCTION=true`, and set the notification URL in
the Production dashboard as well. The server logs a warning if the key and the mode don't match.

### Notes

- After upgrading, journeys that existed before get a price of 0 (free) until an admin sets one,
  and existing bookings are marked as paid.

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
