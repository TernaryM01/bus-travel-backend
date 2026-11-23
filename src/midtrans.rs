//! Midtrans Snap client: payment pages, status lookups, notification signatures.

use std::time::Duration;

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

use crate::config::Config;
use crate::error::AppError;

const SANDBOX_SNAP_URL: &str = "https://app.sandbox.midtrans.com/snap/v1/transactions";
const PRODUCTION_SNAP_URL: &str = "https://app.midtrans.com/snap/v1/transactions";
const SANDBOX_API_URL: &str = "https://api.sandbox.midtrans.com";
const PRODUCTION_API_URL: &str = "https://api.midtrans.com";

const MAX_ITEM_NAME_CHARS: usize = 50;

#[derive(Clone)]
pub struct Midtrans {
    http: reqwest::Client,
    server_key: String,
    snap_url: &'static str,
    api_url: &'static str,
}

pub struct NewTransaction<'a> {
    /// Max 50 characters, unique per attempt
    pub order_id: &'a str,
    pub item_id: String,
    pub item_name: String,
    pub unit_price: i64,
    pub quantity: i32,
    pub customer_name: &'a str,
    pub customer_email: &'a str,
    pub start: DateTime<Utc>,
    pub minutes: i64,
    pub finish_url: String,
}

#[derive(Debug, Deserialize)]
pub struct SnapTransaction {
    pub token: String,
    pub redirect_url: String,
}

/// Result of closing a Snap page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageCancel {
    /// The page can no longer start a payment
    Closed,
    /// A payment method was chosen: there is a transaction to deal with instead
    TransactionInProgress,
}

/// From a notification or the status API.
#[derive(Debug, Clone, Deserialize)]
pub struct TransactionStatus {
    pub order_id: String,
    pub status_code: String,
    pub gross_amount: String,
    pub transaction_status: String,
    #[serde(default)]
    pub fraud_status: Option<String>,
    #[serde(default)]
    pub payment_type: Option<String>,
    #[serde(default)]
    pub signature_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Paid,
    Pending,
    Expired,
    Rejected,
    Ignore,
}

impl TransactionStatus {
    /// https://docs.midtrans.com/docs/https-notification-webhooks
    pub fn outcome(&self) -> Outcome {
        match self.transaction_status.as_str() {
            "settlement" => Outcome::Paid,
            "capture" => match self.fraud_status.as_deref() {
                Some("accept") | None => Outcome::Paid,
                Some("challenge") => Outcome::Pending,
                _ => Outcome::Rejected,
            },
            "pending" => Outcome::Pending,
            "expire" => Outcome::Expired,
            "deny" | "cancel" | "failure" => Outcome::Rejected,
            _ => Outcome::Ignore,
        }
    }

    /// "150000.00" -> 150000
    pub fn amount_rupiah(&self) -> Option<i64> {
        let (whole, fraction) = self.gross_amount.split_once('.').unwrap_or((&self.gross_amount, ""));
        if !fraction.chars().all(|c| c == '0') {
            return None;
        }
        whole.parse().ok()
    }
}

impl Midtrans {
    pub fn new(config: &Config) -> Self {
        let (snap, api) = if config.midtrans_is_production {
            (PRODUCTION_SNAP_URL, PRODUCTION_API_URL)
        } else {
            (SANDBOX_SNAP_URL, SANDBOX_API_URL)
        };

        let sandbox_key = config.midtrans_server_key.starts_with("SB-");
        if config.midtrans_is_production && sandbox_key {
            tracing::warn!("MIDTRANS_IS_PRODUCTION is true, but MIDTRANS_SERVER_KEY looks like a sandbox key");
        } else if !config.midtrans_is_production && !sandbox_key {
            tracing::warn!("Midtrans is in sandbox mode, but MIDTRANS_SERVER_KEY doesn't look like a sandbox key (SB-Mid-server-...)");
        }

        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("Failed to build HTTP client"),
            server_key: config.midtrans_server_key.clone(),
            snap_url: snap,
            api_url: api,
        }
    }

    pub async fn create_transaction(&self, tx: NewTransaction<'_>) -> Result<SnapTransaction, AppError> {
        #[derive(Serialize)]
        struct Body<'a> {
            transaction_details: TransactionDetails<'a>,
            item_details: [Item; 1],
            customer_details: Customer<'a>,
            expiry: Expiry,
            callbacks: Callbacks,
        }
        #[derive(Serialize)]
        struct TransactionDetails<'a> {
            order_id: &'a str,
            gross_amount: i64,
        }
        #[derive(Serialize)]
        struct Item {
            id: String,
            price: i64,
            quantity: i32,
            name: String,
        }
        #[derive(Serialize)]
        struct Customer<'a> {
            first_name: &'a str,
            email: &'a str,
        }
        #[derive(Serialize)]
        struct Expiry {
            start_time: String,
            unit: &'static str,
            duration: i64,
        }
        #[derive(Serialize)]
        struct Callbacks {
            finish: String,
        }

        let gross_amount = tx
            .unit_price
            .checked_mul(i64::from(tx.quantity))
            .ok_or_else(|| AppError::BadRequest("Total harga terlalu besar".to_string()))?;

        let body = Body {
            transaction_details: TransactionDetails { order_id: tx.order_id, gross_amount },
            item_details: [Item {
                id: tx.item_id,
                price: tx.unit_price,
                quantity: tx.quantity,
                name: tx.item_name.chars().take(MAX_ITEM_NAME_CHARS).collect(),
            }],
            customer_details: Customer { first_name: tx.customer_name, email: tx.customer_email },
            // Without start_time, the clock starts when a payment method is picked
            expiry: Expiry {
                start_time: tx.start.format("%Y-%m-%d %H:%M:%S %z").to_string(),
                unit: "minute",
                duration: tx.minutes,
            },
            callbacks: Callbacks { finish: tx.finish_url },
        };

        let response = self
            .http
            .post(self.snap_url)
            .basic_auth(&self.server_key, Some(""))
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await.map_err(|e| {
            tracing::error!("Midtrans Snap request failed: {e}");
            gateway_unavailable()
        })?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            tracing::error!("Midtrans Snap returned {status}: {text}");
            return Err(gateway_unavailable());
        }
        serde_json::from_str(&text).map_err(|e| {
            tracing::error!("Unexpected Midtrans Snap response ({e}): {text}");
            gateway_unavailable()
        })
    }

    /// `None` if no payment method has been chosen yet.
    pub async fn transaction_status(&self, order_id: &str) -> Result<Option<TransactionStatus>, AppError> {
        let url = format!("{}/v2/{}/status", self.api_url, order_id);
        let response = self
            .http
            .get(&url)
            .basic_auth(&self.server_key, Some(""))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                tracing::error!("Midtrans status request failed: {e}");
                gateway_unavailable()
            })?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            tracing::error!("Midtrans status API returned {status}: {text}");
            return Err(gateway_unavailable());
        }

        // An unknown order is HTTP 200 with status_code "404" in the body
        #[derive(Deserialize)]
        struct Code {
            status_code: String,
        }
        match serde_json::from_str::<Code>(&text) {
            Ok(c) if c.status_code == "404" => return Ok(None),
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Unexpected Midtrans status response ({e}): {text}");
                return Err(gateway_unavailable());
            }
        }
        serde_json::from_str(&text).map(Some).map_err(|e| {
            tracing::error!("Unexpected Midtrans status response ({e}): {text}");
            gateway_unavailable()
        })
    }

    /// Stop a pending transaction. Whether it worked is read back with
    /// `transaction_status`: an already-paid transaction can't be expired.
    pub async fn expire(&self, order_id: &str) -> Result<(), AppError> {
        let url = format!("{}/v2/{}/expire", self.api_url, order_id);
        let response = self
            .http
            .post(&url)
            .basic_auth(&self.server_key, Some(""))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                tracing::error!("Midtrans expire request failed: {e}");
                gateway_unavailable()
            })?;
        if response.status().is_server_error() {
            tracing::error!("Midtrans expire returned {}", response.status());
            return Err(gateway_unavailable());
        }
        Ok(())
    }

    /// Close a Snap page, so no payment can be started from it.
    pub async fn cancel_page(&self, token: &str) -> Result<PageCancel, AppError> {
        let url = format!("{}/{}/cancel", self.snap_url, token);
        let response = self
            .http
            .post(&url)
            .basic_auth(&self.server_key, Some(""))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                tracing::error!("Midtrans Snap cancel request failed: {e}");
                gateway_unavailable()
            })?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(PageCancel::Closed);
        }
        #[derive(Deserialize)]
        struct Errors {
            #[serde(default)]
            error_messages: Vec<String>,
        }
        let messages = serde_json::from_str::<Errors>(&text).map(|e| e.error_messages).unwrap_or_default();
        if messages.iter().any(|m| m.eq_ignore_ascii_case("token already canceled")) {
            Ok(PageCancel::Closed)
        } else if messages.iter().any(|m| m.eq_ignore_ascii_case("Transaction is on progress")) {
            Ok(PageCancel::TransactionInProgress)
        } else {
            tracing::error!("Midtrans Snap cancel returned {status}: {text}");
            Err(gateway_unavailable())
        }
    }

    /// signature_key == SHA512(order_id + status_code + gross_amount + server_key)
    pub fn verify_signature(&self, n: &TransactionStatus) -> bool {
        let Some(signature) = n.signature_key.as_deref() else {
            return false;
        };
        let mut hasher = Sha512::new();
        hasher.update(n.order_id.as_bytes());
        hasher.update(n.status_code.as_bytes());
        hasher.update(n.gross_amount.as_bytes());
        hasher.update(self.server_key.as_bytes());
        let expected = hex::encode(hasher.finalize());
        constant_time_eq(expected.as_bytes(), signature.trim().to_ascii_lowercase().as_bytes())
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn gateway_unavailable() -> AppError {
    AppError::Status(
        StatusCode::BAD_GATEWAY,
        "Layanan pembayaran sedang tidak dapat dihubungi. Coba lagi sebentar lagi.".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(transaction_status: &str, fraud: Option<&str>, gross: &str) -> TransactionStatus {
        TransactionStatus {
            order_id: "order-1".into(),
            status_code: "200".into(),
            gross_amount: gross.into(),
            transaction_status: transaction_status.into(),
            fraud_status: fraud.map(Into::into),
            payment_type: None,
            signature_key: None,
        }
    }

    #[test]
    fn outcomes() {
        assert_eq!(status("settlement", None, "1").outcome(), Outcome::Paid);
        assert_eq!(status("capture", Some("accept"), "1").outcome(), Outcome::Paid);
        assert_eq!(status("capture", Some("challenge"), "1").outcome(), Outcome::Pending);
        assert_eq!(status("capture", Some("deny"), "1").outcome(), Outcome::Rejected);
        assert_eq!(status("pending", None, "1").outcome(), Outcome::Pending);
        assert_eq!(status("expire", None, "1").outcome(), Outcome::Expired);
        assert_eq!(status("cancel", None, "1").outcome(), Outcome::Rejected);
        assert_eq!(status("refund", None, "1").outcome(), Outcome::Ignore);
    }

    #[test]
    fn amounts() {
        assert_eq!(status("settlement", None, "150000.00").amount_rupiah(), Some(150000));
        assert_eq!(status("settlement", None, "150000").amount_rupiah(), Some(150000));
        assert_eq!(status("settlement", None, "150000.50").amount_rupiah(), None);
    }

    #[test]
    fn signature() {
        let key = "SB-Mid-server-test";
        let midtrans = Midtrans {
            http: reqwest::Client::new(),
            server_key: key.into(),
            snap_url: "",
            api_url: "",
        };
        let mut n = status("settlement", None, "150000.00");
        let mut hasher = Sha512::new();
        hasher.update(format!("order-1200150000.00{key}"));
        n.signature_key = Some(hex::encode(hasher.finalize()));
        assert!(midtrans.verify_signature(&n));

        n.gross_amount = "1.00".into();
        assert!(!midtrans.verify_signature(&n));
    }
}
