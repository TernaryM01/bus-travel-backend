# Bus Travel API Documentation

## Authentication

All protected endpoints require a JWT token in the Authorization header:

```
Authorization: Bearer <token>
```

Tokens are obtained from the register and login endpoints and expire after 24 hours (configurable).

---

## Response Format

### Success Response
- **Reads and logins** return `200 OK` with a JSON body.
- **Register** returns `201 Created` with a JSON body (the token and user).
- **Other creates** (journeys, bookings) return `201 Created` with no body.
- **Updates and deletes** return `204 No Content` with no body.

### List Order
Every list comes back in a fixed order. Ties are broken by ID.

| List | Order |
|------|-------|
| Journeys (public, driver, admin) | Departure time, latest first |
| Traveller's bookings | Journey departure time, latest first |
| Passengers of a journey (driver, admin) | Booking time, earliest first |
| Cities | ID |
| Users | Account creation time, earliest first |
| Drivers | Name, A–Z |
| Bookings (admin) | Booking time, earliest first |

### Error Response
Every error response has this shape, including malformed requests, unknown routes and rate limiting:
```json
{
  "error": "Perjalanan tidak ditemukan"
}
```
Error messages are in Indonesian and are meant to be shown to the user as they are.

### HTTP Status Codes
| Code | Meaning |
|------|---------|
| 200 | OK - Request succeeded, body included |
| 201 | Created - Resource created |
| 204 | No Content - Update or delete succeeded, no body |
| 400 | Bad Request - Invalid input, malformed JSON, or invalid ID in the URL |
| 401 | Unauthorized - Missing, malformed, invalid or expired token; wrong login |
| 403 | Forbidden - Insufficient permissions |
| 404 | Not Found - Resource or route doesn't exist |
| 405 | Method Not Allowed - Route exists, but not with this HTTP method |
| 409 | Conflict - Resource already exists, or the action isn't allowed in the resource's current state |
| 415 | Unsupported Media Type - Body sent without `Content-Type: application/json` |
| 422 | Unprocessable Entity - JSON body has missing fields or wrong value types |
| 429 | Too Many Requests - Rate limited |
| 500 | Internal Server Error |
| 502 | Bad Gateway - The payment provider (Midtrans) couldn't be reached; try again |

---

## Data Types

### UserRole
```typescript
type UserRole = "Admin" | "Driver" | "Traveller";
```
Role values are case-sensitive, in responses and in request bodies.

### JourneyStatus
```typescript
type JourneyStatus = "Scheduled" | "PickingUp" | "InTransit" | "Finished";
```

### PickupStatus
```typescript
type PickupStatus = "Pending" | "PickedUp" | "NoShow";
```

### PaymentStatus
```typescript
type PaymentStatus = "Pending" | "Paid" | "Expired" | "Rejected" | "Failed" | "Cancelled";
```
- `Pending`: waiting for payment. Holds its seats, also past the deadline, until Midtrans gives a final answer.
- `Paid`: confirmed (also bookings on free journeys, and bookings made before payments existed).
- `Expired`: Midtrans confirmed it can no longer be paid. Seats released.
- `Rejected`: declined, cancelled or errored at Midtrans. Seats released.
- `Failed`: still `Pending` when pickup started; whether it was paid is unknown (check the order in Midtrans).
- `Cancelled`: cancelled by the traveller before paying. Seats released.

At the deadline the backend stops the payment at Midtrans (or finds it paid). Until Midtrans
confirms, the booking stays `Pending`.
Only `Paid` bookings are passengers (driver and admin passenger lists, pickup statuses).

### Payment
Returned when booking, paying, or checking a payment.
```typescript
interface Payment {
  booking_id: string;
  payment_status: PaymentStatus;
  amount: number;                    // Total in whole rupiah (IDR)
  payment_url: string | null;        // Midtrans payment page; set while Pending
  payment_expires_at: string | null; // Deadline to pay; set while Pending
}
```

### City
```typescript
interface City {
  id: number;
  name: string;           // "Kupang" or "Soe"
  center_lat: number;     // Latitude of city center
  center_lng: number;     // Longitude of city center
  pickup_radius_km: number; // Max pickup distance from center
  station_name: string;   // Drop-off station: where journeys arriving in this city end
  station_lat: number;    // Latitude of the station
  station_lng: number;    // Longitude of the station
}
```

### User
```typescript
interface User {
  id: string;             // UUID
  email: string;
  name: string;
  role: UserRole;
  created_at: string;     // ISO 8601 datetime
}
```

### Journey
```typescript
interface Journey {
  id: string;             // UUID
  origin_city_id: number;
  destination_city_id: number;
  departure_time: string; // ISO 8601 datetime
  total_seats: number;
  driver_id: string | null; // UUID or null if unassigned
  created_at: string;
  status: JourneyStatus;
  price_per_seat: number;   // Whole rupiah; 0 = free
}
```

### Booking
```typescript
interface Booking {
  id: string;             // UUID
  journey_id: string;
  user_id: string;
  seats: number;
  pickup_lat: number;
  pickup_lng: number;
  created_at: string;
  pickup_status: PickupStatus;
  payment_status: PaymentStatus;
  amount: number;                    // Total in whole rupiah, fixed by the latest payment attempt
  payment_order_id: string | null;   // Midtrans order id of the latest payment attempt
  payment_url: string | null;
  payment_expires_at: string | null;
  paid_at: string | null;
  payment_method: string | null;     // Midtrans payment_type: "bank_transfer", "qris", "gopay", ...
}
```

---

## Public Endpoints

### Register Traveller

Creates a new traveller account.

```
POST /api/auth/register
```

**Request Body:**
```json
{
  "email": "user@example.com",
  "password": "password123",
  "name": "John Doe"
}
```

**Response:** `201 Created`
```json
{
  "token": "eyJhbGciOiJIUzI1NiIs...",
  "user": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "email": "user@example.com",
    "name": "John Doe",
    "role": "Traveller"
  }
}
```

**Errors:**
- `409 Conflict`: Email already registered

---

### Login

Authenticates a user and returns a JWT token.

```
POST /api/auth/login
```

**Request Body:**
```json
{
  "email": "user@example.com",
  "password": "password123"
}
```

**Response:** `200 OK`
```json
{
  "token": "eyJhbGciOiJIUzI1NiIs...",
  "user": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "email": "user@example.com",
    "name": "John Doe",
    "role": "Traveller"
  }
}
```

**Errors:**
- `401 Unauthorized`: Invalid email or password, or the account only uses Google sign-in

---

### Login with Google

Signs in with a Google ID token (from Google Identity Services). On first sign-in this creates a `Traveller` account, or links Google to an existing account with the same email.

```
POST /api/auth/google
```

**Request Body:**
```json
{
  "credential": "<Google ID token>"
}
```

**Response:** `200 OK` — same format as Login.

**Errors:**
- `401 Unauthorized`: Invalid Google token, or a token issued for a different client ID (`OAUTH_CLIENT_ID`)

---

### List Available Journeys

Returns journeys that haven't departed yet and whose status is still `Scheduled`, including full ones (`available_seats: 0`).
Seats held by bookings that are still waiting for payment don't count as available.

```
GET /api/journeys
```

**Response:** `200 OK`
```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "origin_city": {
      "id": 1,
      "name": "Kupang",
      "center_lat": -10.1836,
      "center_lng": 123.6257,
      "pickup_radius_km": 5.0,
      "station_name": "Pusat Kota Kupang",
      "station_lat": -10.1836,
      "station_lng": 123.6257
    },
    "destination_city": {
      "id": 2,
      "name": "Soe",
      "center_lat": -9.8684,
      "center_lng": 124.2861,
      "pickup_radius_km": 2.0,
      "station_name": "Pusat Kota Soe",
      "station_lat": -9.8684,
      "station_lng": 124.2861
    },
    "departure_time": "2024-01-15T08:00:00Z",
    "available_seats": 35,
    "has_driver": true,
    "price_per_seat": 150000
  }
]
```

---

### Get Journey Details

```
GET /api/journeys/{id}
```

Returns any journey by ID, including past ones.

**Response:** `200 OK` — same format as a list item above.

**Errors:**
- `404 Not Found`: Journey not found

---

## Traveller Endpoints

*Requires authentication with the `Traveller` role.*

### Book a Journey

```
POST /api/bookings
```

**Request Body:**
```json
{
  "journey_id": "550e8400-e29b-41d4-a716-446655440000",
  "seats": 2,
  "pickup_lat": -10.18,
  "pickup_lng": 123.62
}
```

On a journey with a price, the booking is created as `Pending` and holds its seats; the traveller
has until `payment_expires_at` (`PAYMENT_HOLD_MINUTES`, default 30) to pay. Send the traveller to `payment_url`
(the Midtrans payment page). Afterwards Midtrans sends them back to `<FRONTEND_URL>/my-bookings`
with `?order_id=...&transaction_status=...` added. On a free journey (`price_per_seat` 0) the booking
is `Paid` straight away and `payment_url` is `null`.

If the traveller already has an `Expired`, `Rejected`, `Failed` or `Cancelled` booking on this journey, it is replaced by
the new one (same booking `id`).

**Response:** `201 Created` — a [Payment](#payment)
```json
{
  "booking_id": "660e8400-e29b-41d4-a716-446655440001",
  "payment_status": "Pending",
  "amount": 300000,
  "payment_url": "https://app.sandbox.midtrans.com/snap/v4/redirection/...",
  "payment_expires_at": "2024-01-10T11:00:00Z"
}
```

**Errors:**
- `400 Bad Request`:
  - Fewer than 1 seat
  - Not enough seats available
  - Past journey
  - Pickup point outside allowed radius
- `404 Not Found`: Journey not found
- `409 Conflict`:
  - Already booked this journey (paid, or payment still pending)
  - Journey is no longer `Scheduled` (the driver has started picking up passengers)
- `502 Bad Gateway`: Midtrans couldn't be reached; nothing was booked

---

### List My Bookings

```
GET /api/bookings
```

**Response:** `200 OK`
```json
[
  {
    "id": "660e8400-e29b-41d4-a716-446655440001",
    "journey_id": "550e8400-e29b-41d4-a716-446655440000",
    "origin_city": "Kupang",
    "destination_city": "Soe",
    "departure_time": "2024-01-15T08:00:00Z",
    "journey_status": "Scheduled",
    "seats": 2,
    "pickup_lat": -10.18,
    "pickup_lng": 123.62,
    "created_at": "2024-01-10T10:30:00Z",
    "payment_status": "Paid",
    "amount": 300000,
    "payment_expires_at": null,
    "paid_at": "2024-01-10T10:41:12Z",
    "payment_method": "bank_transfer"
  }
]
```

`payment_expires_at` is set only while the payment is `Pending`.

Paid bookings can't be cancelled by travellers. An admin can delete one (see [Delete Booking](#delete-booking-admin)).

---

### Pay for a Booking

```
POST /api/bookings/{id}/pay
```

- `Pending`: returns the current payment page. Past the deadline, asks Midtrans first.
- `Expired` or `Rejected`: starts a new payment attempt (new `payment_url` and deadline), priced at
  the journey's current price, if the seats are still free and the journey still takes bookings.

**Response:** `200 OK` — a [Payment](#payment)

**Errors:**
- `404 Not Found`: Booking not found (or not yours)
- `409 Conflict`:
  - Already paid
  - Past the deadline and Midtrans hasn't given a final answer yet
  - Journey no longer takes bookings (departed, or pickup has started)
  - The seats were taken by someone else in the meantime
- `502 Bad Gateway`: Midtrans couldn't be reached

---

### Cancel an Unpaid Booking

```
POST /api/bookings/{id}/cancel
```

Stops the payment at Midtrans. If Midtrans confirms it's stopped, the booking becomes `Cancelled`
and its seats are free at once. If the payment went through just before, the booking becomes
`Paid` instead (check `payment_status` in the response).

**Response:** `200 OK` — a [Payment](#payment)

**Errors:**
- `404 Not Found`: Booking not found (or not yours)
- `409 Conflict`: Booking isn't waiting for payment (paid, expired, rejected, failed or already cancelled)
- `502 Bad Gateway`: Midtrans couldn't be reached or gave no definite answer; nothing changed

---

### Check Payment Status

```
POST /api/bookings/{id}/payment/refresh
```

Asks Midtrans for the payment's status now and records it, instead of waiting for Midtrans's
notification. Call it when the traveller comes back from the payment page. It is also how
payments get confirmed in local development, where Midtrans can't reach the backend.

**Response:** `200 OK` — a [Payment](#payment)

**Errors:**
- `404 Not Found`: Booking not found (or not yours)
- `502 Bad Gateway`: Midtrans couldn't be reached

---

## Payment Notifications

### Midtrans Notification

```
POST /api/payments/midtrans/notification
```

Called by Midtrans whenever a payment's status changes. Set it as **Payment Notification URL**
in the Midtrans dashboard (Settings > Configuration), e.g.
`https://api.example.com/api/payments/midtrans/notification`. No login: each notification is
checked against its `signature_key`, `SHA512(order_id + status_code + gross_amount + server key)`.

| Midtrans `transaction_status` | Booking |
|---|---|
| `settlement`, `capture` (fraud status `accept`) | `Paid` (if it was `Pending` and the amount matches) |
| `pending`, `capture` (fraud status `challenge`) | unchanged |
| `expire` | `Expired` (if `Pending`) |
| `deny`, `cancel`, `failure` | `Rejected` (if `Pending`) |
| `refund`, `partial_refund` | unchanged |

**Response:** `200 OK`

**Errors:**
- `401 Unauthorized`: Wrong signature
- `404 Not Found`: No booking has this order id. Midtrans tries again later, which covers a
  notification that arrives before its booking was saved.

---

## Driver Endpoints

*Requires authentication with the `Driver` role.*

### List My Assigned Journeys

```
GET /api/driver/journeys
```

**Response:** `200 OK`
```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "origin_city": {
      "id": 1,
      "name": "Kupang",
      "center_lat": -10.1836,
      "center_lng": 123.6257,
      "pickup_radius_km": 5.0,
      "station_name": "Pusat Kota Kupang",
      "station_lat": -10.1836,
      "station_lng": 123.6257
    },
    "destination_city": {
      "id": 2,
      "name": "Soe",
      "center_lat": -9.8684,
      "center_lng": 124.2861,
      "pickup_radius_km": 2.0,
      "station_name": "Pusat Kota Soe",
      "station_lat": -9.8684,
      "station_lng": 124.2861
    },
    "departure_time": "2024-01-15T08:00:00Z",
    "total_seats": 40,
    "booked_seats": 25,
    "status": "Scheduled"
  }
]
```

---

### Get Passenger Pickup Points

```
GET /api/driver/journeys/{id}/passengers
```

Lists the journey's passengers: its `Paid` bookings.

**Response:** `200 OK`
```json
{
  "journey_id": "550e8400-e29b-41d4-a716-446655440000",
  "origin_city": "Kupang",
  "destination_city": "Soe",
  "departure_time": "2024-01-15T08:00:00Z",
  "status": "PickingUp",
  "passengers": [
    {
      "booking_id": "660e8400-e29b-41d4-a716-446655440001",
      "passenger_name": "John Doe",
      "seats": 2,
      "pickup_lat": -10.18,
      "pickup_lng": 123.62,
      "pickup_status": "PickedUp"
    },
    {
      "booking_id": "660e8400-e29b-41d4-a716-446655440002",
      "passenger_name": "Jane Smith",
      "seats": 1,
      "pickup_lat": -10.19,
      "pickup_lng": 123.63,
      "pickup_status": "Pending"
    }
  ]
}
```

**Errors:**
- `403 Forbidden`: Not assigned to this journey
- `404 Not Found`: Journey not found

---

### Update Journey Status

Move the journey one stage forwards or backwards: `Scheduled` ↔ `PickingUp` ↔ `InTransit` ↔ `Finished`. Going back undoes a mistaken tap.

```
PUT /api/driver/journeys/{id}/status
```

**Request Body:**
```json
{
  "status": "PickingUp"
}
```

**Response:** `204 No Content`

**Errors:**
- `403 Forbidden`: Not assigned to this journey
- `404 Not Found`: Journey not found
- `409 Conflict`:
  - Not exactly one stage away from the current status (skipping a stage, or the same status)
  - Moving to `InTransit` while a passenger's pickup status is still `Pending`
- `422 Unprocessable Entity`: Unknown status value

---

### Update Passenger Pickup Status

Set a passenger's pickup status to any of `Pending`, `PickedUp` or `NoShow`. Only while the journey is `PickingUp`.

```
PUT /api/driver/journeys/{id}/passengers/{booking_id}/pickup-status
```

**Request Body:**
```json
{
  "pickup_status": "PickedUp"
}
```

**Response:** `204 No Content`

**Errors:**
- `403 Forbidden`: Not assigned to this journey
- `404 Not Found`: Journey not found, or booking not found in this journey
- `409 Conflict`: Journey is not `PickingUp`
- `422 Unprocessable Entity`: Unknown pickup status value

---

## Admin Endpoints

*Requires authentication with the `Admin` role.*

### List Cities

```
GET /api/admin/cities
```

**Response:** `200 OK`
```json
[
  {
    "id": 1,
    "name": "Kupang",
    "center_lat": -10.1836,
    "center_lng": 123.6257,
    "pickup_radius_km": 5.0,
    "station_name": "Pusat Kota Kupang",
    "station_lat": -10.1836,
    "station_lng": 123.6257
  },
  {
    "id": 2,
    "name": "Soe",
    "center_lat": -9.8684,
    "center_lng": 124.2861,
    "pickup_radius_km": 2.0,
    "station_name": "Pusat Kota Soe",
    "station_lat": -9.8684,
    "station_lng": 124.2861
  }
]
```

---

### Update City Station

Change the name and location of a city's drop-off station. All three fields are required.

```
PUT /api/admin/cities/{id}/station
```

**Request Body:**
```json
{
  "station_name": "Terminal Kupang",
  "station_lat": -10.17,
  "station_lng": 123.61
}
```

`station_name` is trimmed of surrounding spaces before saving. The station may be anywhere: it doesn't have to be inside the city's pickup radius.

**Response:** `204 No Content`

**Errors:**
- `400 Bad Request`:
  - Empty station name
  - Station name longer than 100 characters
  - Latitude outside -90 to 90, or longitude outside -180 to 180
- `404 Not Found`: City not found

---

### List All Journeys

Returns all journeys with driver info, seat counts and prices. `booked_seats` counts paid
bookings plus bookings still waiting for payment (they hold their seats).

```
GET /api/admin/journeys
```

**Response:** `200 OK`
```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "origin_city": "Kupang",
    "destination_city": "Soe",
    "departure_time": "2024-01-15T08:00:00Z",
    "total_seats": 40,
    "booked_seats": 25,
    "driver": {
      "id": "770e8400-e29b-41d4-a716-446655440003",
      "name": "Driver One",
      "email": "driver1@example.com"
    },
    "status": "Scheduled",
    "price_per_seat": 150000
  }
]
```

---

### Create Journey

```
POST /api/admin/journeys
```

**Request Body:**
```json
{
  "origin_city_id": 1,
  "destination_city_id": 2,
  "departure_time": "2024-01-15T08:00:00Z",
  "total_seats": 40,
  "price_per_seat": 150000
}
```

`price_per_seat` is in whole rupiah, from 0 (free: bookings are confirmed without payment) to `MAX_PRICE_PER_SEAT`.

**Response:** `201 Created` (no body)

**Errors:**
- `400 Bad Request`: Invalid city ID, same origin/destination, or price out of range

---

### Update Journey

```
PUT /api/admin/journeys/{id}
```

**Request Body:** (all fields optional)
```json
{
  "origin_city_id": 1,
  "destination_city_id": 2,
  "departure_time": "2024-01-15T09:00:00Z",
  "total_seats": 45,
  "price_per_seat": 175000
}
```

A new price applies to bookings made, or paid again, afterwards. Paid bookings and payments in progress keep their amount.

**Response:** `204 No Content`

**Errors:**
- `400 Bad Request`: Invalid city ID, or price out of range
- `404 Not Found`: Journey not found

---

### Delete Journey

```
DELETE /api/admin/journeys/{id}
```

**Response:** `204 No Content`

> ⚠️ Deleting a journey also deletes all associated bookings (cascade).

---

### Assign Driver to Journey

```
POST /api/admin/journeys/{id}/assign-driver
```

**Request Body:**
```json
{
  "driver_id": "770e8400-e29b-41d4-a716-446655440003"
}
```

**Response:** `204 No Content`

**Errors:**
- `400 Bad Request`: User is not a driver
- `404 Not Found`: Driver or journey not found

---

### Get Passenger Pickup Points (Admin)

Same as the driver endpoint of the same name, for any journey (no assignment check).

```
GET /api/admin/journeys/{id}/passengers
```

**Response:** `200 OK` — same format as the driver version.

**Errors:**
- `404 Not Found`: Journey not found

---

### List All Users

Returns all user accounts with their roles.

```
GET /api/admin/users
```

**Response:** `200 OK`
```json
[
  {
    "id": "770e8400-e29b-41d4-a716-446655440003",
    "email": "user@example.com",
    "name": "User Name",
    "role": "Traveller",
    "created_at": "2024-01-01T00:00:00Z"
  }
]
```

---

### List All Drivers

Returns users with the `Driver` role.

```
GET /api/admin/drivers
```

**Response:** `200 OK`
```json
[
  {
    "id": "770e8400-e29b-41d4-a716-446655440003",
    "email": "driver1@example.com",
    "name": "Driver One",
    "created_at": "2024-01-01T00:00:00Z"
  }
]
```

---

### Update User Role

Change any user's role to `Admin`, `Driver` or `Traveller`.

```
PUT /api/admin/users/{id}/role
```

**Request Body:**
```json
{
  "role": "Driver"
}
```

**Response:** `204 No Content`

> ⚠️ When changing from driver role, user is unassigned from all journeys. When changing from traveller role, user's bookings are deleted.

**Errors:**
- `404 Not Found`: User not found
- `422 Unprocessable Entity`: Unknown role value (role names are case-sensitive)

---

### Delete User Account

Delete any user account (including other admins), except the caller's own.

```
DELETE /api/admin/users/{id}
```

**Response:** `204 No Content`

> ⚠️ When deleted: drivers are unassigned from journeys, travellers have their bookings deleted.

**Errors:**
- `403 Forbidden`: `{id}` is the logged-in admin's own account
- `404 Not Found`: User not found

---

### List All Bookings

```
GET /api/admin/bookings
```

**Response:** `200 OK`
```json
[
  {
    "id": "660e8400-e29b-41d4-a716-446655440001",
    "journey_id": "550e8400-e29b-41d4-a716-446655440000",
    "user_name": "John Doe",
    "user_email": "john@example.com",
    "seats": 2,
    "pickup_lat": -10.18,
    "pickup_lng": 123.62,
    "created_at": "2024-01-10T10:30:00Z",
    "payment_status": "Paid",
    "amount": 300000,
    "paid_at": "2024-01-10T10:41:12Z",
    "payment_method": "qris",
    "payment_order_id": "660e8400e29b41d4a716446655440001-3f9a1c2e"
  }
]
```

`payment_order_id` is the Midtrans order id of the latest payment attempt: search for it in the
Midtrans dashboard to see the transaction or refund it.

---

### Delete Booking (Admin)

Delete any booking. Money already paid is **not** refunded automatically: refund it in the
Midtrans dashboard (by `payment_order_id`) or by other means.

```
DELETE /api/admin/bookings/{id}
```

**Response:** `204 No Content`

**Errors:**
- `404 Not Found`: Booking not found

---

### Update Booking (Admin)

Modify a booking's pickup point and/or number of seats. Admin can put pickup point outside of the allowed circle area and can overbook (exceed the number of seats available). The amount paid (or being paid) doesn't change.

```
PUT /api/admin/bookings/{id}
```

**Request Body:** (all fields optional)
```json
{
  "pickup_lat": -10.17,
  "pickup_lng": 123.64,
  "seats": 3
}
```

**Response:** `204 No Content`

**Errors:**
- `400 Bad Request`: Seats must be positive
- `404 Not Found`: Booking not found

---

## Rate Limiting

The API uses a **two-layer rate limiting** system:

Each limit is a token bucket: a client can send up to the burst size at once, then refills at the steady rate.

### Layer 1: Global Rate Limit (IP-based)

Applied to **all requests** before authentication.

| Steady rate | Burst | Key |
|-------------|-------|-----|
| 500 requests per minute | 1000 | IP address |

**Purpose**: Protects against DDoS and abuse from unauthenticated sources.

### Layer 2: Route Rate Limits

| Routes | Steady rate | Burst | Key |
|--------|-------------|-------|-----|
| Public (`/api/auth/*`, `/api/journeys`, `/api/journeys/{id}`) | 50 requests per minute | 100 | IP address |
| Driver | 250 requests per minute | 500 | User ID |
| Traveller | 50 requests per minute | 100 | User ID |

Admin routes and Midtrans notifications (`/api/payments/*`) only have the global limit.

**Purpose**: Provides fair usage limits based on account type. Per-user keying ensures users behind shared IPs (NAT, corporate networks) each get their own quota.

### Response When Rate Limited

- **Status**: `429 Too Many Requests`
- **Body**: `{"error": "Terlalu banyak permintaan. Silakan coba lagi sebentar lagi."}`

---

## Frontend Integration Notes

### Storing the Token
After login/register, store the token securely (e.g., `localStorage` or `httpOnly` cookie) and include it in all subsequent requests.

### Handling Token Expiration
Tokens expire after 24 hours. When a `401 Unauthorized` response is received, redirect the user to the login page.

### Map Integration
For the pickup point selection:
1. Take the origin city from the journey (`journey.origin_city`), which includes its center coordinates and allowed radius. (The full city list, `GET /api/admin/cities`, is admin-only.)
2. Display a map centered on the origin city
3. Draw a circle with the `pickup_radius_km` to show the valid area
4. Validate the selected point is within the radius before submitting

### Role-Based UI
Use the `role` field from the login response to show/hide features:
- **Traveller**: Journey list, booking, my bookings
- **Driver**: Assigned journeys, passenger pickup map
- **Admin**: Journey CRUD, user management (list/role/delete), booking management (view/delete/update)
