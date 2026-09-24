//! Data carried from one request to the next (validation errors and flash messages), and
//! the redirect responses that produce it. Storage is left to a store middleware (see
//! `cookie_flash_middleware` behind the `cookie-flash` feature): it puts the data read from
//! the previous request in [`IncomingFlash`] and persists [`OutgoingFlash`] from the response.

use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::BoxError;

/// Errors and flash data that survive one redirect.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlashData {
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub errors: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub flash: Map<String, Value>,
    /// Clear the client's (encrypted) history on the next page, e.g. after logging out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_history: bool,
}

impl FlashData {
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty() && self.flash.is_empty() && !self.clear_history
    }
}

/// Request extension: data flashed by the previous request.
#[derive(Debug, Clone, Default)]
pub struct IncomingFlash(pub FlashData);

/// Response extension: data to hand to the next request.
#[derive(Debug, Clone, Default)]
pub struct OutgoingFlash(pub FlashData);

/// Response extension set when a page was rendered, so the store knows the incoming flash
/// data has been shown and can be dropped.
#[derive(Debug, Clone, Copy)]
pub struct FlashConsumed;

/// A redirect that can carry validation errors and flash data to the next page.
/// Create it with `Inertia::redirect` or `Inertia::back`.
#[derive(Debug)]
#[must_use]
pub struct InertiaRedirect {
    location: String,
    error_bag: Option<String>,
    data: FlashData,
    error: Option<String>,
}

impl InertiaRedirect {
    pub(crate) fn new(location: String, error_bag: Option<String>) -> Self {
        Self {
            location,
            error_bag,
            data: FlashData::default(),
            error: None,
        }
    }

    /// Validation errors keyed by field: a message or a list of messages per field.
    /// They end up in `props.errors` (under `props.errors.<bag>` when the request that
    /// failed validation named an error bag).
    pub fn with_errors(mut self, errors: impl Serialize) -> Self {
        match to_object(errors) {
            Ok(errors) => match &self.error_bag {
                Some(bag) if !errors.is_empty() => {
                    let entry = self
                        .data
                        .errors
                        .entry(bag.clone())
                        .or_insert_with(|| Value::Object(Map::new()));
                    match entry {
                        // Earlier `with_errors` calls filled the bag: add to it.
                        Value::Object(existing) => existing.extend(errors),
                        other => *other = Value::Object(errors),
                    }
                }
                _ => self.data.errors.extend(errors),
            },
            Err(e) => self.error = Some(format!("invalid validation errors: {e}")),
        }
        self
    }

    /// Clear the client's history on the page this redirect leads to (e.g. after logout),
    /// so the back button cannot reveal pages of the previous session.
    pub fn clear_history(mut self) -> Self {
        self.data.clear_history = true;
        self
    }

    /// One-time data for the next page, available as `page.flash`.
    pub fn with_flash(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        match serde_json::to_value(value) {
            Ok(v) => {
                self.data.flash.insert(key.into(), v);
            }
            Err(e) => self.error = Some(format!("invalid flash value: {e}")),
        }
        self
    }
}

impl IntoResponse for InertiaRedirect {
    fn into_response(self) -> Response {
        if let Some(e) = self.error {
            tracing::error!("{e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let Ok(location) = HeaderValue::from_str(&self.location) else {
            tracing::error!("invalid redirect location {:?}", self.location);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        // 302; the Inertia middleware turns it into 303 after PUT/PATCH/DELETE.
        let mut res = (StatusCode::FOUND, [(header::LOCATION, location)]).into_response();
        if !self.data.is_empty() {
            res.extensions_mut().insert(OutgoingFlash(self.data));
        }
        res
    }
}

pub(crate) fn to_object(value: impl Serialize) -> Result<Map<String, Value>, BoxError> {
    match serde_json::to_value(value)? {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(Map::new()),
        other => Err(format!("expected a JSON object, got {other}").into()),
    }
}
