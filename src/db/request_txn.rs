//! Every request that may write (anything but GET, HEAD, OPTIONS) runs as one
//! SERIALIZABLE transaction, so concurrent requests can't interfere with each other
//! (two travellers booking the last seat, two clicks on "register", ...). Handlers
//! don't need any code for it: they keep using `state.db`, and while a request's
//! transaction is open, `CachedDb` sends every query of that request through it.
//!
//! How it stays correct:
//! - Postgres runs these transactions concurrently, and aborts one with a
//!   serialization failure (SQLSTATE 40001) when the outcome would differ from
//!   running them one after the other. Deadlocks (40P01) are handled the same way.
//! - A request whose transaction was aborted is run again from the start, with the
//!   same method, URL, headers and body, up to `MAX_ATTEMPTS` times. So handlers may
//!   be run more than once: anything they do outside the database must be safe to
//!   repeat (calling Google to verify a token is; sending an email would not be).
//! - The transaction is committed only when the response is a success (2xx or 3xx).
//!   An error response rolls back everything the request wrote.
//! - Reads in a write request skip the read cache (they must see the transaction's
//!   own view). Its writes invalidate the cache when it commits.
//!
//! GET, HEAD and OPTIONS requests don't get a transaction and are answered from the
//! read cache as before. They must not write to the database.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rand::Rng;
use sea_orm::{DbErr, RuntimeErr};

use super::cached::{CachedDb, CachedTxn};
use crate::error::AppError;

/// Runs per request, including the first. After the last one fails, the client
/// gets a 503 and may try again. With the pauses below, all attempts together take
/// a few seconds at most.
const MAX_ATTEMPTS: u32 = 10;

/// Request bodies are kept in memory to run a request again. Same limit as axum's
/// default for `Json` bodies.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

tokio::task_local! {
    /// The transaction of the request being handled by the current task, if any.
    static CURRENT: Arc<Scope>;
}

pub(crate) struct Scope {
    txn: CachedTxn,
    /// Set when a query of this attempt failed with a serialization failure or
    /// deadlock: the attempt is then rolled back and run again, whatever the
    /// handler made of the error.
    conflicted: AtomicBool,
}

impl Scope {
    pub(crate) fn inner(&self) -> &CachedTxn {
        &self.txn
    }

    /// Pass a query's result through, noting a serialization failure.
    pub(crate) fn track<T>(&self, result: Result<T, DbErr>) -> Result<T, DbErr> {
        if let Err(e) = &result {
            if is_conflict(e) {
                self.conflicted.store(true, SeqCst);
            }
        }
        result
    }
}

/// The transaction of the write request this code is running for, if any.
pub(crate) fn current_write_request() -> Option<Arc<Scope>> {
    CURRENT.try_with(Arc::clone).ok()
}

/// Serialization failure or deadlock: the transaction can succeed if run again.
pub(crate) fn is_conflict(e: &DbErr) -> bool {
    match e {
        DbErr::Conn(RuntimeErr::SqlxError(sea_orm::sqlx::Error::Database(e)))
        | DbErr::Exec(RuntimeErr::SqlxError(sea_orm::sqlx::Error::Database(e)))
        | DbErr::Query(RuntimeErr::SqlxError(sea_orm::sqlx::Error::Database(e))) => {
            matches!(e.code().as_deref(), Some("40001" | "40P01"))
        }
        _ => false,
    }
}

/// Middleware: run each write request in a SERIALIZABLE transaction, and run it
/// again when Postgres reports a conflict with another request.
pub async fn transaction_per_request(
    State(db): State<CachedDb>,
    request: Request,
    next: Next,
) -> Response {
    if matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    let Ok(body) = axum::body::to_bytes(body, MAX_BODY_BYTES).await else {
        return AppError::Status(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Data yang dikirim terlalu besar".to_string(),
        )
        .into_response();
    };

    for attempt in 1..=MAX_ATTEMPTS {
        let request = Request::from_parts(parts.clone(), Body::from(body.clone()));
        let txn = match db.begin_serializable().await {
            Ok(txn) => txn,
            Err(e) => return AppError::from(e).into_response(),
        };
        let scope = Arc::new(Scope { txn, conflicted: AtomicBool::new(false) });
        let response = CURRENT.scope(Arc::clone(&scope), next.clone().run(request)).await;

        // Every query of the handler has finished, so this is the last reference.
        let Ok(scope) = Arc::try_unwrap(scope) else {
            tracing::error!("Request transaction still in use after the handler returned");
            return AppError::Internal("request transaction leaked".to_string()).into_response();
        };

        let conflicted = scope.conflicted.load(SeqCst);
        let outcome = if conflicted {
            let _ = scope.txn.rollback().await;
            None
        } else if response.status().is_success() || response.status().is_redirection() {
            match scope.txn.commit().await {
                Ok(()) => Some(response),
                // Postgres may also detect the conflict only at commit
                Err(e) if is_conflict(&e) => None,
                Err(e) => Some(AppError::from(e).into_response()),
            }
        } else {
            // An error response: undo whatever the request wrote
            let _ = scope.txn.rollback().await;
            Some(response)
        };

        match outcome {
            Some(response) => return response,
            None if attempt < MAX_ATTEMPTS => {
                tracing::debug!("Request conflicted with another one; running it again ({attempt})");
                // A random pause that grows with each attempt (10 ms, 20 ms, ... up
                // to 640 ms at most), so requests that collided don't collide again
                let max_ms = 10u64 << (attempt - 1).min(6);
                let pause = rand::thread_rng().gen_range(0..=max_ms);
                tokio::time::sleep(Duration::from_millis(pause)).await;
            }
            None => {}
        }
    }

    tracing::warn!("Request still conflicting after {MAX_ATTEMPTS} attempts");
    AppError::Status(
        StatusCode::SERVICE_UNAVAILABLE,
        "Server sedang sibuk. Silakan coba lagi.".to_string(),
    )
    .into_response()
}
