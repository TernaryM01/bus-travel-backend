//! Drop-in replacements for `axum::Json` and `axum::extract::Path`.
//!
//! They behave exactly like axum's versions, except that when a request can't
//! be parsed (wrong Content-Type, malformed JSON, missing field, bad UUID in the
//! URL, ...) the error is returned as `{"error": "..."}` like every other error
//! in this API, instead of axum's default plain-text response.

use axum::extract::{FromRequest, FromRequestParts};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::error::AppError;

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
pub struct Json<T>(pub T);

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(AppError))]
pub struct Path<T>(pub T);
