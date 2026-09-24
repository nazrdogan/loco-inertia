//! Flash storage in an encrypted, HTTP-only cookie (`cookie-flash` feature).

use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, Key, PrivateCookieJar, SameSite};

use crate::flash::{FlashConsumed, FlashData, IncomingFlash, OutgoingFlash};

pub use axum_extra::extract::cookie::Key as CookieKey;

/// Configuration for [`cookie_flash_middleware`], provided via `Extension<CookieFlash>`.
///
/// The cookie is encrypted and authenticated with `key`, so clients can neither read nor
/// forge it. Keep the payload small: browsers cap cookies at about 4 KB.
#[derive(Clone)]
pub struct CookieFlash {
    key: Key,
    cookie_name: String,
    secure: bool,
}

impl CookieFlash {
    pub fn new(key: Key) -> Self {
        Self {
            key,
            cookie_name: "inertia_flash".to_string(),
            secure: false,
        }
    }

    /// Mark the cookie `Secure` (HTTPS only). Enable it in production.
    #[must_use]
    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    #[must_use]
    pub fn cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = name.into();
        self
    }
}

impl std::fmt::Debug for CookieFlash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CookieFlash")
            .field("cookie_name", &self.cookie_name)
            .field("secure", &self.secure)
            .finish_non_exhaustive()
    }
}

/// Reads flash data flashed by the previous request and stores data flashed by this one.
///
/// Incoming data is dropped once a page has been rendered with it; requests that render
/// nothing (asset version conflicts, other redirects, non-Inertia routes) leave it in place.
pub async fn cookie_flash_middleware(mut req: Request, next: Next) -> Response {
    let Some(store) = req.extensions().get::<CookieFlash>().cloned() else {
        return next.run(req).await;
    };
    let jar = PrivateCookieJar::from_headers(req.headers(), store.key.clone());
    let incoming = jar
        .get(&store.cookie_name)
        .and_then(|c| serde_json::from_str::<FlashData>(c.value()).ok());
    if let Some(data) = &incoming {
        req.extensions_mut().insert(IncomingFlash(data.clone()));
    }

    let mut res = next.run(req).await;
    let consumed = res.extensions_mut().remove::<FlashConsumed>().is_some();
    let outgoing = res.extensions_mut().remove::<OutgoingFlash>();

    let jar = match outgoing {
        // Sent as measured: `fitted_jar` holds the encrypted cookie whose size it checked.
        Some(OutgoingFlash(data)) if !data.is_empty() => match store.fitted_jar(data) {
            Some(jar) => jar,
            None => return res,
        },
        _ if incoming.is_some() && consumed => jar.remove(store.cookie(String::new())),
        _ => return res,
    };
    (jar, res).into_response()
}

/// Largest `Set-Cookie` header we emit. Browsers drop cookies over about 4 KB (name, value
/// and attributes) without telling anyone, which would lose the errors silently.
pub const MAX_FLASH_COOKIE_BYTES: usize = 4000;

/// A named way to make flash data smaller.
type Shrink = (&'static str, fn(&mut FlashData));

/// Messages are cut to this many characters when the data does not fit.
const TRUNCATED_MESSAGE_CHARS: usize = 200;

/// Replaces the errors that do not fit, under this key.
pub const OVERFLOW_ERROR_KEY: &str = "_overflow";

impl CookieFlash {
    /// A jar holding the encrypted cookie for `data`, shrunk step by step until its
    /// `Set-Cookie` header fits in [`MAX_FLASH_COOKIE_BYTES`]: first message per field,
    /// shorter messages, no flash data, then only the error fields that fit plus an
    /// [`OVERFLOW_ERROR_KEY`] notice. Every step is logged, so an oversized payload never
    /// disappears silently.
    ///
    /// Each encryption uses a fresh nonce and the cookie encoding percent-encodes some base64
    /// characters, so the size varies between encryptions of the same data: the jar that was
    /// measured is the one that gets sent.
    fn fitted_jar(&self, data: FlashData) -> Option<PrivateCookieJar> {
        let (jar, original) = self.encrypt(&data)?;
        if original <= MAX_FLASH_COOKIE_BYTES {
            return Some(jar);
        }
        let mut data = data;
        let steps: [Shrink; 3] = [
            ("kept only the first message per field", first_messages),
            ("shortened the messages", truncate_messages),
            ("dropped the flash data", |d| d.flash.clear()),
        ];
        for (step, shrink) in steps {
            shrink(&mut data);
            let (jar, len) = self.encrypt(&data)?;
            tracing::warn!(original, now = len, "flash cookie too large: {step}");
            if len <= MAX_FLASH_COOKIE_BYTES {
                return Some(jar);
            }
        }
        // Keep the error fields that fit, and say that there were more.
        let all = std::mem::take(&mut data.errors);
        let total = all.len();
        data.errors.insert(
            OVERFLOW_ERROR_KEY.to_string(),
            serde_json::Value::String("There are more errors than can be shown.".to_string()),
        );
        let mut kept = Vec::new();
        for (field, message) in all {
            data.errors.insert(field.clone(), message);
            if self.encrypt(&data)?.1 > MAX_FLASH_COOKIE_BYTES {
                data.errors.remove(&field);
            } else {
                kept.push(field);
            }
        }
        // The last check used another encryption than the one we send: re-measure, and drop
        // fields from the end until the jar actually sent fits.
        loop {
            let (jar, len) = self.encrypt(&data)?;
            if len <= MAX_FLASH_COOKIE_BYTES {
                tracing::error!(
                    original,
                    kept = kept.len(),
                    total,
                    "flash cookie too large: dropped validation errors that did not fit"
                );
                return Some(jar);
            }
            let field = kept.pop()?;
            data.errors.remove(&field);
        }
    }

    /// Encrypt `data` into a jar, with the length of the `Set-Cookie` header it produces.
    fn encrypt(&self, data: &FlashData) -> Option<(PrivateCookieJar, usize)> {
        let json = match serde_json::to_string(data) {
            Ok(json) => json,
            Err(e) => {
                tracing::error!("failed to serialize flash data: {e}");
                return None;
            }
        };
        let jar = PrivateCookieJar::new(self.key.clone()).add(self.cookie(json));
        let len = (jar.clone(), ())
            .into_response()
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .map_or(usize::MAX, |v| v.as_bytes().len());
        Some((jar, len))
    }

    fn cookie(&self, value: String) -> Cookie<'static> {
        Cookie::build((self.cookie_name.clone(), value))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax)
            .secure(self.secure)
            .build()
    }
}

/// `{"field": ["a", "b"]}` → `{"field": "a"}`.
fn first_messages(data: &mut FlashData) {
    for value in data.errors.values_mut() {
        first_message(value);
    }
}

fn first_message(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            let first = items.drain(..).next().unwrap_or_default();
            *value = first;
            first_message(value);
        }
        // An error bag: `{"bag": {"field": [...]}}`.
        serde_json::Value::Object(map) => map.values_mut().for_each(first_message),
        _ => {}
    }
}

fn truncate_messages(data: &mut FlashData) {
    data.errors.values_mut().for_each(truncate);
    data.flash.values_mut().for_each(truncate);
}

fn truncate(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) if s.chars().count() > TRUNCATED_MESSAGE_CHARS => {
            let cut: String = s.chars().take(TRUNCATED_MESSAGE_CHARS).collect();
            *s = format!("{cut}…");
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(truncate),
        serde_json::Value::Object(map) => map.values_mut().for_each(truncate),
        _ => {}
    }
}
