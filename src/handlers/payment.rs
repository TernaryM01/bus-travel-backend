//! Booking payments. Safe to re-run (see db::request_txn): every attempt gets a
//! new order id, and pages from rolled-back attempts are never shown.
//!
//! Seats are released only after Midtrans confirms no money can arrive anymore:
//! at the deadline (a timer per booking) or when the traveller cancels, the
//! payment is stopped at Midtrans, or found to be already paid.

use axum::{extract::State, http::StatusCode, Extension};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use rand::Rng;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveEnum, ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Serialize;
use uuid::Uuid;

use crate::entities::booking::{self, PaymentStatus};
use crate::entities::journey::{self, JourneyStatus};
use crate::entities::{city, user};
use crate::error::{AppError, AppResult};
use crate::extract::{Json, Path};
use crate::handlers::shared;
use crate::handlers::traveller::BOOKING_CUTOFF;
use crate::midtrans::{NewTransaction, Outcome, PageCancel, TransactionStatus};
use crate::utils::jwt::Claims;
use crate::AppState;

/// WITA, for departure times in item names
const WITA_OFFSET_SECS: i32 = 8 * 3600;

/// Wait before asking Midtrans again when it gave no definite answer
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Serialize)]
pub struct PaymentResponse {
    pub booking_id: Uuid,
    pub payment_status: PaymentStatus,
    pub amount: i64,
    pub payment_url: Option<String>,
    pub payment_expires_at: Option<DateTime<Utc>>,
}

impl PaymentResponse {
    pub fn of(b: &booking::Model) -> Self {
        let pending = b.payment_status == PaymentStatus::Pending;
        Self {
            booking_id: b.id,
            payment_status: b.payment_status,
            amount: b.amount,
            payment_url: if pending { b.payment_url.clone() } else { None },
            payment_expires_at: if pending {
                b.payment_expires_at.map(|t| t.with_timezone(&Utc))
            } else {
                None
            },
        }
    }
}

pub struct PaymentAttempt {
    pub order_id: String,
    pub token: String,
    pub url: String,
    pub expires_at: DateTime<Utc>,
}

impl PaymentAttempt {
    /// Also starts the deadline timer (a no-op if the request is rolled back).
    pub fn apply_to(self, state: &AppState, active: &mut booking::ActiveModel, booking_id: Uuid, amount: i64) {
        schedule_deadline(state.clone(), booking_id, self.order_id.clone(), self.expires_at);
        active.payment_status = Set(PaymentStatus::Pending);
        active.amount = Set(amount);
        active.payment_order_id = Set(Some(self.order_id));
        active.payment_token = Set(Some(self.token));
        active.payment_url = Set(Some(self.url));
        active.payment_expires_at = Set(Some(self.expires_at.into()));
        active.paid_at = Set(None);
        active.payment_method = Set(None);
    }
}

pub async fn new_payment_attempt(
    state: &AppState,
    booking_id: Uuid,
    journey: &journey::Model,
    seats: i32,
    traveller: &user::Model,
) -> AppResult<PaymentAttempt> {
    let cities = city::Entity::find().all(&state.db).await?;
    let name_of = |id: i32| {
        cities.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default()
    };
    let wita = FixedOffset::east_opt(WITA_OFFSET_SECS).expect("valid offset");
    let departure = journey.departure_time.with_timezone(&wita);

    // Midtrans rejects reused order ids
    let order_id = format!("{}-{:08x}", booking_id.simple(), rand::thread_rng().r#gen::<u32>());
    let start = Utc::now();
    let minutes = state.config.payment_hold_minutes;

    let snap = state
        .midtrans
        .create_transaction(NewTransaction {
            order_id: &order_id,
            item_id: journey.id.simple().to_string(),
            item_name: format!(
                "Tiket {}-{} {} WITA",
                name_of(journey.origin_city_id),
                name_of(journey.destination_city_id),
                departure.format("%d-%m-%Y, %H:%M")
            ),
            unit_price: journey.price_per_seat,
            quantity: seats,
            customer_name: &traveller.name,
            customer_email: &traveller.email,
            start,
            minutes,
            finish_url: format!("{}/my-bookings", state.config.frontend_url),
        })
        .await?;

    Ok(PaymentAttempt {
        order_id,
        token: snap.token,
        url: snap.redirect_url,
        expires_at: start + Duration::minutes(minutes),
    })
}

pub fn total_price(journey: &journey::Model, seats: i32) -> AppResult<i64> {
    journey
        .price_per_seat
        .checked_mul(i64::from(seats))
        .ok_or_else(|| AppError::BadRequest("Total harga terlalu besar".to_string()))
}

/// Midtrans's definite answer about a payment.
enum Final {
    Paid(TransactionStatus),
    /// No money can arrive anymore
    Closed(PaymentStatus),
}

/// Stop a pending payment at Midtrans, or find out it already went through.
/// An error means no definite answer yet: the seats must stay held.
async fn stop_payment(state: &AppState, booking: &booking::Model, closed_as: PaymentStatus) -> AppResult<Final> {
    let (Some(order_id), Some(token)) = (booking.payment_order_id.as_deref(), booking.payment_token.as_deref()) else {
        return Err(AppError::Internal(format!("Pending booking {} has no Midtrans order", booking.id)));
    };
    for _ in 0..3 {
        match state.midtrans.transaction_status(order_id).await? {
            Some(status) => match status.outcome() {
                Outcome::Paid => return Ok(Final::Paid(status)),
                Outcome::Expired => return Ok(Final::Closed(closed_as)),
                Outcome::Rejected => return Ok(Final::Closed(PaymentStatus::Rejected)),
                Outcome::Pending => state.midtrans.expire(order_id).await?,
                Outcome::Ignore => break,
            },
            // No payment method chosen yet: close the page, then make sure no
            // payment started in the meantime
            None => match state.midtrans.cancel_page(token).await? {
                PageCancel::Closed => {
                    if state.midtrans.transaction_status(order_id).await?.is_none() {
                        return Ok(Final::Closed(closed_as));
                    }
                }
                PageCancel::TransactionInProgress => {}
            },
        }
    }
    Err(AppError::Status(
        StatusCode::BAD_GATEWAY,
        "Status pembayaran belum pasti. Coba lagi sebentar lagi.".to_string(),
    ))
}

/// Record Midtrans's answer, if the booking is still waiting for this order.
async fn record(state: &AppState, booking: &booking::Model, answer: Final) -> AppResult<()> {
    let update = booking::Entity::update_many()
        .filter(booking::Column::Id.eq(booking.id))
        .filter(booking::Column::PaymentStatus.eq(PaymentStatus::Pending))
        .filter(booking::Column::PaymentOrderId.eq(booking.payment_order_id.clone()));
    let update = match answer {
        Final::Paid(status) => {
            if status.amount_rupiah() != Some(booking.amount) {
                // Seats stay held: money may have been taken
                tracing::error!(
                    "Midtrans reports order {} paid with {}, but booking {} costs {}; left pending for an admin",
                    status.order_id, status.gross_amount, booking.id, booking.amount
                );
                return Ok(());
            }
            update
                .col_expr(booking::Column::PaymentStatus, PaymentStatus::Paid.as_enum())
                .col_expr(booking::Column::PaidAt, Expr::value(Some(DateTime::<FixedOffset>::from(Utc::now()))))
                .col_expr(booking::Column::PaymentMethod, Expr::value(status.payment_type))
        }
        Final::Closed(status) => update.col_expr(booking::Column::PaymentStatus, status.as_enum()),
    };
    update.exec(&state.db).await?;
    Ok(())
}

/// Apply a status Midtrans reported (notification or "Cek status").
async fn apply_status(state: &AppState, booking: &booking::Model, status: TransactionStatus) -> AppResult<()> {
    if booking.payment_status != PaymentStatus::Pending {
        if status.outcome() == Outcome::Paid && !booking.is_paid() {
            tracing::error!(
                "Midtrans reports order {} paid, but booking {} is {:?}",
                status.order_id, booking.id, booking.payment_status
            );
        }
        return Ok(());
    }
    match status.outcome() {
        Outcome::Paid => record(state, booking, Final::Paid(status)).await,
        Outcome::Expired => record(state, booking, Final::Closed(PaymentStatus::Expired)).await,
        Outcome::Rejected => record(state, booking, Final::Closed(PaymentStatus::Rejected)).await,
        Outcome::Pending | Outcome::Ignore => Ok(()),
    }
}

/// Fires at a payment's deadline. Runs again after errors until Midtrans answers.
fn schedule_deadline(state: AppState, booking_id: Uuid, order_id: String, at: DateTime<Utc>) {
    tokio::spawn(async move {
        let wait = (at - Utc::now()).to_std().unwrap_or_default();
        tokio::time::sleep(wait).await;
        loop {
            match close_at_deadline(&state, booking_id, &order_id).await {
                Ok(()) => return,
                Err(e) => {
                    tracing::warn!("Closing payment of booking {booking_id} failed, retrying: {e}");
                    tokio::time::sleep(RETRY_AFTER).await;
                }
            }
        }
    });
}

async fn close_at_deadline(state: &AppState, booking_id: Uuid, order_id: &str) -> AppResult<()> {
    let Some(booking) = booking::Entity::find_by_id(booking_id).one(&state.db).await? else {
        return Ok(());
    };
    if booking.payment_status != PaymentStatus::Pending || booking.payment_order_id.as_deref() != Some(order_id) {
        return Ok(());
    }
    let answer = stop_payment(state, &booking, PaymentStatus::Expired).await?;
    record(state, &booking, answer).await
}

/// On startup: timers for every booking still waiting for payment.
pub async fn schedule_pending_deadlines(state: &AppState) -> AppResult<()> {
    let pending = booking::Entity::find()
        .filter(booking::Column::PaymentStatus.eq(PaymentStatus::Pending))
        .all(&state.db)
        .await?;
    for b in pending {
        if let (Some(order_id), Some(at)) = (b.payment_order_id, b.payment_expires_at) {
            schedule_deadline(state.clone(), b.id, order_id, at.with_timezone(&Utc));
        }
    }
    Ok(())
}

async fn own_booking(state: &AppState, claims: &Claims, booking_id: Uuid) -> AppResult<booking::Model> {
    booking::Entity::find_by_id(booking_id)
        .filter(booking::Column::UserId.eq(claims.sub))
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Pesanan tidak ditemukan".to_string()))
}

fn past_deadline(booking: &booking::Model) -> bool {
    booking.payment_expires_at.is_some_and(|d| d.with_timezone(&Utc) <= Utc::now())
}

pub async fn pay_booking(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(booking_id): Path<Uuid>,
) -> AppResult<Json<PaymentResponse>> {
    let booking = own_booking(&state, &claims, booking_id).await?;
    let now = Utc::now();

    match booking.payment_status {
        PaymentStatus::Paid => {
            return Err(AppError::Conflict("Pesanan ini sudah dibayar".to_string()));
        }
        PaymentStatus::Cancelled => {
            return Err(AppError::Conflict("Pesanan ini sudah dibatalkan".to_string()));
        }
        PaymentStatus::Pending if past_deadline(&booking) => {
            return Err(AppError::Conflict(
                "Pembayaran Anda masih dikonfirmasi. Cek lagi sebentar lagi".to_string(),
            ));
        }
        PaymentStatus::Pending => return Ok(Json(PaymentResponse::of(&booking))),
        PaymentStatus::Expired | PaymentStatus::Rejected | PaymentStatus::Failed => {}
    }

    let journey = journey::Entity::find_by_id(booking.journey_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Perjalanan tidak ditemukan".to_string()))?;
    if journey.departure_time.with_timezone(&Utc) <= now + BOOKING_CUTOFF || journey.status != JourneyStatus::Scheduled {
        return Err(AppError::Conflict(
            "Perjalanan ini sudah tidak menerima pemesanan".to_string(),
        ));
    }

    let available = journey.total_seats - shared::seats_taken(&state.db, journey.id, Some(booking.id)).await?;
    if booking.seats > available {
        return Err(AppError::Conflict(format!(
            "Kursi sudah dipesan orang lain. Tersisa {} kursi",
            available.max(0)
        )));
    }

    let amount = total_price(&journey, booking.seats)?;
    let mut active: booking::ActiveModel = booking.clone().into();
    if amount == 0 {
        active.payment_status = Set(PaymentStatus::Paid);
        active.amount = Set(0);
        active.paid_at = Set(Some(now.into()));
    } else {
        let traveller = user::Entity::find_by_id(claims.sub)
            .one(&state.db)
            .await?
            .ok_or_else(|| AppError::Unauthorized("Akun tidak ditemukan".to_string()))?;
        new_payment_attempt(&state, booking.id, &journey, booking.seats, &traveller)
            .await?
            .apply_to(&state, &mut active, booking.id, amount);
    }
    let updated = active.update(&state.db).await?;

    Ok(Json(PaymentResponse::of(&updated)))
}

/// Cancel an unpaid booking: its seats are free right away, once Midtrans
/// confirms the payment is stopped. If it turns out paid, the response says so.
pub async fn cancel_booking(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(booking_id): Path<Uuid>,
) -> AppResult<Json<PaymentResponse>> {
    let booking = own_booking(&state, &claims, booking_id).await?;
    if booking.payment_status != PaymentStatus::Pending {
        return Err(AppError::Conflict(match booking.payment_status {
            PaymentStatus::Paid => "Pesanan yang sudah dibayar tidak dapat dibatalkan".to_string(),
            _ => "Pesanan ini sudah tidak aktif".to_string(),
        }));
    }

    let answer = stop_payment(&state, &booking, PaymentStatus::Cancelled).await?;
    record(&state, &booking, answer).await?;

    let updated = booking::Entity::find_by_id(booking.id)
        .one(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Pesanan tidak ditemukan".to_string()))?;
    Ok(Json(PaymentResponse::of(&updated)))
}

/// Ask Midtrans for the payment's status now
pub async fn refresh_payment(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(booking_id): Path<Uuid>,
) -> AppResult<Json<PaymentResponse>> {
    let booking = own_booking(&state, &claims, booking_id).await?;
    if booking.payment_status == PaymentStatus::Pending {
        if let Some(order_id) = booking.payment_order_id.as_deref() {
            if let Some(status) = state.midtrans.transaction_status(order_id).await? {
                apply_status(&state, &booking, status).await?;
            }
        }
    }
    let booking = own_booking(&state, &claims, booking_id).await?;
    Ok(Json(PaymentResponse::of(&booking)))
}

pub async fn midtrans_notification(
    State(state): State<AppState>,
    Json(notification): Json<TransactionStatus>,
) -> AppResult<StatusCode> {
    if !state.midtrans.verify_signature(&notification) {
        tracing::warn!("Rejected a payment notification with a wrong signature (order {})", notification.order_id);
        return Err(AppError::Unauthorized("Tanda tangan notifikasi tidak valid".to_string()));
    }

    let Some(booking) = booking::Entity::find()
        .filter(booking::Column::PaymentOrderId.eq(notification.order_id.as_str()))
        .one(&state.db)
        .await?
    else {
        // 404 makes Midtrans retry, in case the booking isn't committed yet
        tracing::info!("Payment notification for unknown order {}", notification.order_id);
        return Err(AppError::NotFound("Pesanan tidak ditemukan".to_string()));
    };

    tracing::info!(
        "Payment notification: order {} is {}",
        notification.order_id, notification.transaction_status
    );
    apply_status(&state, &booking, notification).await?;

    Ok(StatusCode::OK)
}
