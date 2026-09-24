//! Server-side implementation of the [Inertia.js v3 protocol](https://inertiajs.com/docs/v3)
//! for axum. This crate knows nothing about Loco (or any other framework built on axum).
//!
//! Wiring:
//!
//! ```no_run
//! use axum::{routing::get, Extension, Router};
//! use inertia_core::{inertia_middleware, Inertia, InertiaConfig};
//!
//! async fn home(inertia: Inertia) -> axum::response::Response {
//!     inertia.render("Home", serde_json::json!({ "greeting": "hi" })).await
//! }
//!
//! let config = InertiaConfig::new(|view: &inertia_core::RootView<'_>| {
//!     Ok(format!("<!doctype html><html><body>{}</body></html>", view.body))
//! });
//! let app: Router = Router::new()
//!     .route("/", get(home))
//!     // `Extension` must wrap the middleware so the middleware can see the config.
//!     .layer(axum::middleware::from_fn(inertia_middleware))
//!     .layer(Extension(config));
//! ```

mod config;
#[cfg(feature = "cookie-flash")]
mod cookie_flash;
#[cfg(feature = "csrf")]
mod csrf;
mod extractor;
mod flash;
pub mod headers;
mod middleware;
mod page;
mod props;
mod scroll;
mod shared;
mod ssr;

pub use config::{BoxError, InertiaConfig, RootTemplate, RootView};
#[cfg(feature = "cookie-flash")]
pub use cookie_flash::{
    cookie_flash_middleware, CookieFlash, CookieKey, MAX_FLASH_COOKIE_BYTES, OVERFLOW_ERROR_KEY,
};
#[cfg(feature = "csrf")]
pub use csrf::{csrf_middleware, Csrf, CSRF_MISMATCH};
pub use extractor::{Inertia, InertiaRejection};
pub use flash::{FlashConsumed, FlashData, IncomingFlash, InertiaRedirect, OutgoingFlash};
pub use middleware::inertia_middleware;
pub use page::{mount_markup, to_script_json, Page};
pub use props::{InertiaPage, IntoProp, IntoProps, Prop, PropRequest, Props, Resolved};
pub use scroll::{MergeIntent, ScrollData, ScrollMeta};
pub use shared::SharedProps;
#[cfg(feature = "ssr")]
pub use ssr::HttpSsr;
pub use ssr::{
    CircuitBreaker, SsrFuture, SsrRenderer, SsrResponse, DEFAULT_SSR_COOLDOWN,
    DEFAULT_SSR_FAILURE_THRESHOLD,
};
