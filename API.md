# Bus Travel API Documentation

Base URL: `http://localhost:3000`

## Authentication

All protected endpoints require a JWT token in the Authorization header:

```
Authorization: Bearer <token>
```

Tokens are obtained from the login endpoint and expire after 24 hours (configurable).

---

## Response Format

### Success Response
```json
{
  "id": "uuid",
  "field": "value"
}
```

### Error Response
```json
{
  "error": "Error message"
}
```

### HTTP Status Codes
| Code | Meaning |
|------|---------|
| 200 | Success |
| 400 | Bad Request - Invalid input |
| 401 | Unauthorized - Invalid/missing token |
| 403 | Forbidden - Insufficient permissions |
| 404 | Not Found |
| 409 | Conflict - Resource already exists |
| 429 | Too Many Requests - Rate limited |
| 500 | Internal Server Error |

---

## Data Types

### UserRole
```typescript
type UserRole = "admin" | "driver" | "traveller";
```

### City
```typescript
interface City {
  id: number;
  name: string;           // "Kupang" or "Bandung"
  center_lat: number;     // Latitude of city center
  center_lng: number;     // Longitude of city center
  pickup_radius_km: number; // Max pickup distance from center
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

**Response:** `200 OK`
```json
{
  "token": "eyJhbGciOiJIUzI1NiIs...",
  "user": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "email": "user@example.com",
    "name": "John Doe",
    "role": "traveller"
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
    "role": "traveller"
  }
}
```

**Errors:**
- `401 Unauthorized`: Invalid email or password

---

### List Available Journeys

Returns future journeys with available seats.

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
      "center_lat": -6.2088,
      "center_lng": 106.8456,
      "pickup_radius_km": 10.0
    },
    "destination_city": {
      "id": 2,
      "name": "Bandung",
      "center_lat": -6.9175,
      "center_lng": 107.6191,
      "pickup_radius_km": 7.0
    },
    "departure_time": "2024-01-15T08:00:00Z",
    "available_seats": 35,
    "has_driver": true
  }
]
```

---

### Get Journey Details

```
GET /api/journeys/{id}
```

**Response:** Same format as list item above.

---

### List Cities

```
GET /api/cities
```

**Response:** `200 OK`
```json
[
  {
    "id": 1,
    "name": "Kupang",
    "center_lat": -6.2088,
    "center_lng": 106.8456,
    "pickup_radius_km": 10.0
  },
  {
    "id": 2,
    "name": "Bandung",
    "center_lat": -6.9175,
    "center_lng": 107.6191,
    "pickup_radius_km": 7.0
  }
]
```

---

## Traveller Endpoints

*Requires authentication with `traveller` role.*

### Book a Journey

```
POST /api/bookings
```

**Request Body:**
```json
{
  "journey_id": "550e8400-e29b-41d4-a716-446655440000",
  "seats": 2,
  "pickup_lat": -6.21,
  "pickup_lng": 106.85
}
```

**Response:** `200 OK`
```json
{
  "id": "660e8400-e29b-41d4-a716-446655440001",
  "journey_id": "550e8400-e29b-41d4-a716-446655440000",
  "origin_city": "Kupang",
  "destination_city": "Bandung",
  "departure_time": "2024-01-15T08:00:00Z",
  "seats": 2,
  "pickup_lat": -6.21,
  "pickup_lng": 106.85,
  "created_at": "2024-01-10T10:30:00Z"
}
```

**Errors:**
- `400 Bad Request`: 
  - Not enough seats available
  - Past journey
  - Pickup point outside allowed radius
- `404 Not Found`: Journey not found
- `409 Conflict`: Already booked this journey

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
    "destination_city": "Bandung",
    "departure_time": "2024-01-15T08:00:00Z",
    "seats": 2,
    "pickup_lat": -6.21,
    "pickup_lng": 106.85,
    "created_at": "2024-01-10T10:30:00Z"
  }
]
```

---

### Cancel Booking

```
DELETE /api/bookings/{id}
```

**Response:** `200 OK`
```json
{
  "message": "Booking cancelled"
}
```

**Errors:**
- `400 Bad Request`: Cannot cancel past journey bookings
- `403 Forbidden`: Not your booking
- `404 Not Found`: Booking not found

---

## Driver Endpoints

*Requires authentication with `driver` role.*

### List My Assigned Journeys

```
GET /api/driver/journeys
```

**Response:** `200 OK`
```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "origin_city": "Kupang",
    "destination_city": "Bandung",
    "departure_time": "2024-01-15T08:00:00Z",
    "total_seats": 40,
    "booked_seats": 25
  }
]
```

---

### Get Passenger Pickup Points

```
GET /api/driver/journeys/{id}/passengers
```

**Response:** `200 OK`
```json
{
  "journey_id": "550e8400-e29b-41d4-a716-446655440000",
  "origin_city": "Kupang",
  "destination_city": "Bandung",
  "departure_time": "2024-01-15T08:00:00Z",
  "passengers": [
    {
      "booking_id": "660e8400-e29b-41d4-a716-446655440001",
      "passenger_name": "John Doe",
      "seats": 2,
      "pickup_lat": -6.21,
      "pickup_lng": 106.85
    },
    {
      "booking_id": "660e8400-e29b-41d4-a716-446655440002",
      "passenger_name": "Jane Smith",
      "seats": 1,
      "pickup_lat": -6.19,
      "pickup_lng": 106.82
    }
  ]
}
```

**Errors:**
- `403 Forbidden`: Not assigned to this journey
- `404 Not Found`: Journey not found

---

## Admin Endpoints

*Requires authentication with `admin` role.*

### List All Journeys

Returns all journeys with driver info and seat counts.

```
GET /api/admin/journeys
```

**Response:** `200 OK`
```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "origin_city": "Kupang",
    "destination_city": "Bandung",
    "departure_time": "2024-01-15T08:00:00Z",
    "total_seats": 40,
    "booked_seats": 25,
    "driver": {
      "id": "770e8400-e29b-41d4-a716-446655440003",
      "name": "Driver One",
      "email": "driver1@example.com"
    }
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
  "total_seats": 40
}
```

**Response:** `200 OK`
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "origin_city_id": 1,
  "destination_city_id": 2,
  "departure_time": "2024-01-15T08:00:00Z",
  "total_seats": 40,
  "driver_id": null,
  "created_at": "2024-01-10T10:30:00Z"
}
```

**Errors:**
- `400 Bad Request`: Invalid city ID or same origin/destination

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
  "total_seats": 45
}
```

**Response:** Updated journey object.

---

### Delete Journey

```
DELETE /api/admin/journeys/{id}
```

**Response:** `200 OK`
```json
{
  "message": "Journey deleted"
}
```

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

**Response:** Updated journey object with driver_id set.

**Errors:**
- `400 Bad Request`: User is not a driver
- `404 Not Found`: Driver or journey not found

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
    "role": "traveller",
    "created_at": "2024-01-01T00:00:00Z"
  }
]
```

---

### List All Drivers

Returns users with driver role.

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

Change any user's role (admin, driver, or traveller).

```
PUT /api/admin/users/{id}/role
```

**Request Body:**
```json
{
  "role": "driver"
}
```

**Response:** `200 OK`
```json
{
  "id": "770e8400-e29b-41d4-a716-446655440003",
  "email": "user@example.com",
  "name": "User Name",
  "role": "driver",
  "created_at": "2024-01-01T00:00:00Z"
}
```

> **Note:** When changing from driver role, user is unassigned from all journeys. When changing from traveller role, user's bookings are deleted.

**Errors:**
- `404 Not Found`: User not found

---

### Delete User Account

Delete any user account (including admins).

```
DELETE /api/admin/users/{id}
```

**Response:** `200 OK`
```json
{
  "message": "User deleted"
}
```

> **Note:** Drivers are unassigned from journeys. Users with bookings have their bookings deleted.

**Errors:**
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
    "pickup_lat": -6.21,
    "pickup_lng": 106.85,
    "created_at": "2024-01-10T10:30:00Z"
  }
]
```

---

### Delete Booking (Admin)

Delete any booking.

```
DELETE /api/admin/bookings/{id}
```

**Response:** `200 OK`
```json
{
  "message": "Booking deleted"
}
```

**Errors:**
- `404 Not Found`: Booking not found

---

### Update Booking (Admin)

Modify a booking's pickup point and/or number of seats. Admin can put pickup point outside of the allowed circle area and can overbook (exceed the number of seats available).

```
PUT /api/admin/bookings/{id}
```

**Request Body:** (all fields optional)
```json
{
  "pickup_lat": -6.22,
  "pickup_lng": 106.84,
  "seats": 3
}
```

**Response:** `200 OK`
```json
{
  "id": "660e8400-e29b-41d4-a716-446655440001",
  "journey_id": "550e8400-e29b-41d4-a716-446655440000",
  "user_name": "John Doe",
  "user_email": "john@example.com",
  "seats": 3,
  "pickup_lat": -6.22,
  "pickup_lng": 106.84,
  "created_at": "2024-01-10T10:30:00Z"
}
```

**Errors:**
- `404 Not Found`: Booking not found

---

## Rate Limiting

The API is rate-limited to **100 requests per 60 seconds** per IP address.

When exceeded, the server responds with:
- **Status**: `429 Too Many Requests`

---

## Frontend Integration Notes

### Storing the Token
After login/register, store the token securely (e.g., `localStorage` or `httpOnly` cookie) and include it in all subsequent requests.

### Handling Token Expiration
Tokens expire after 24 hours. When a `401 Unauthorized` response is received, redirect the user to the login page.

### Map Integration
For the pickup point selection:
1. Fetch cities from `/api/cities` to get center coordinates and allowed radius
2. Display a map centered on the origin city
3. Draw a circle with the `pickup_radius_km` to show the valid area
4. Validate the selected point is within the radius before submitting

### Role-Based UI
Use the `role` field from the login response to show/hide features:
- **traveller**: Journey list, booking, my bookings
- **driver**: Assigned journeys, passenger pickup map
- **admin**: Journey CRUD, user management (list/role/delete), booking management (view/delete/update)

