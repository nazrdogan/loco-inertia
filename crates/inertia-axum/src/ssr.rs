//! Server-side rendering of first visits. [`SsrRenderer`] turns a page into HTML; the
//! `ssr` feature provides [`HttpSsr`] for the Inertia Node server
//! (`createServer` from `@inertiajs/react/server` and friends). Any failure falls back to
//! client-side rendering.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{BoxError, Page};

/// What [`SsrRenderer::render`] returns.
pub type SsrFuture<'a> = Pin<Box<dyn Future<Output = Result<SsrResponse, BoxError>> + Send + 'a>>;

/// What an SSR server returns for a page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SsrResponse {
    /// Tags for `<head>` (from `<Head>` components).
    #[serde(default)]
    pub head: Vec<String>,
    /// Replaces the mount markup: page data script plus the server-rendered root element.
    pub body: String,
}

/// Renders a page on the server. Only called for first (non-Inertia) visits.
pub trait SsrRenderer: Send + Sync {
    /// Render `page` to HTML; an error makes the page render client-side.
    fn render<'a>(&'a self, page: &'a Page) -> SsrFuture<'a>;
}

/// Stops calling a failing renderer for a while, so a hung or crashed SSR server costs one
/// timeout per few requests instead of one per first visit.
///
/// After `threshold` consecutive failures the circuit opens: pages render client-side
/// without trying SSR. After `cooldown`, one request probes the renderer again; success closes
/// the circuit, failure reopens it. `InertiaConfig::with_ssr` wraps every renderer in one.
pub struct CircuitBreaker<R> {
    inner: R,
    threshold: u32,
    cooldown: Duration,
    failures: AtomicU32,
    open_until: Mutex<Option<Instant>>,
    probing: AtomicBool,
}

/// Consecutive SSR failures before [`CircuitBreaker`] opens.
pub const DEFAULT_SSR_FAILURE_THRESHOLD: u32 = 3;
/// How long [`CircuitBreaker`] skips SSR once open.
pub const DEFAULT_SSR_COOLDOWN: Duration = Duration::from_secs(30);

impl<R: SsrRenderer> CircuitBreaker<R> {
    /// Wrap `inner`: open after `threshold` failures in a row (at least 1), for `cooldown`.
    pub fn new(inner: R, threshold: u32, cooldown: Duration) -> Self {
        Self {
            inner,
            threshold: threshold.max(1),
            cooldown,
            failures: AtomicU32::new(0),
            open_until: Mutex::new(None),
            probing: AtomicBool::new(false),
        }
    }

    /// Whether this request may call the renderer; `Some(true)` marks the half-open probe.
    fn permit(&self) -> Option<bool> {
        let open_until = *self.open_until.lock().unwrap_or_else(|e| e.into_inner());
        match open_until {
            None => Some(false),
            Some(until) if Instant::now() < until => None,
            // Cooldown over: let exactly one request through to test the renderer.
            Some(_) => self
                .probing
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .ok()
                .map(|_| true),
        }
    }

    fn succeeded(&self) {
        self.failures.store(0, Ordering::Release);
        let mut open_until = self.open_until.lock().unwrap_or_else(|e| e.into_inner());
        if open_until.take().is_some() {
            tracing::info!("SSR recovered; server-side rendering resumes");
        }
    }

    fn failed(&self, probe: bool) {
        let failures = self.failures.fetch_add(1, Ordering::AcqRel) + 1;
        if probe || failures >= self.threshold {
            let mut open_until = self.open_until.lock().unwrap_or_else(|e| e.into_inner());
            if open_until.is_none() || probe {
                tracing::warn!(
                    failures,
                    cooldown_secs = self.cooldown.as_secs_f32(),
                    "SSR keeps failing; rendering client-side until the cooldown ends"
                );
            }
            *open_until = Some(Instant::now() + self.cooldown);
        }
    }
}

impl<R: SsrRenderer> SsrRenderer for CircuitBreaker<R> {
    fn render<'a>(&'a self, page: &'a Page) -> SsrFuture<'a> {
        Box::pin(async move {
            let Some(probe) = self.permit() else {
                return Err("SSR circuit open (the renderer failed repeatedly)".into());
            };
            // The future is dropped when the client disconnects mid-render; without this the
            // probe flag would stay set and no request could ever probe again.
            let _guard = probe.then(|| ProbeGuard(&self.probing));
            let result = self.inner.render(page).await;
            match &result {
                Ok(_) => self.succeeded(),
                Err(_) => self.failed(probe),
            }
            result
        })
    }
}

/// Frees the half-open probe slot however the probe ends, including cancellation.
struct ProbeGuard<'a>(&'a AtomicBool);

impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(feature = "ssr")]
pub use http::HttpSsr;

#[cfg(feature = "ssr")]
mod http {
    use std::time::Duration;

    use super::{SsrFuture, SsrRenderer, SsrResponse};
    use crate::Page;

    /// Client for the Inertia SSR server: `POST {url}/render` with the page as JSON.
    #[derive(Debug, Clone)]
    pub struct HttpSsr {
        client: reqwest::Client,
        render_url: String,
        timeout: Duration,
    }

    impl HttpSsr {
        /// Where the Inertia SSR server listens by default.
        pub const DEFAULT_URL: &'static str = "http://127.0.0.1:13714";

        /// `url` is the server's base URL, e.g. [`HttpSsr::DEFAULT_URL`].
        pub fn new(url: impl AsRef<str>) -> Self {
            Self {
                client: reqwest::Client::new(),
                render_url: format!("{}/render", url.as_ref().trim_end_matches('/')),
                timeout: Duration::from_secs(2),
            }
        }

        /// Give up (and render client-side) after this long. Defaults to 2 seconds.
        #[must_use]
        pub fn with_timeout(mut self, timeout: Duration) -> Self {
            self.timeout = timeout;
            self
        }
    }

    impl SsrRenderer for HttpSsr {
        fn render<'a>(&'a self, page: &'a Page) -> SsrFuture<'a> {
            Box::pin(async move {
                let res = self
                    .client
                    .post(&self.render_url)
                    .timeout(self.timeout)
                    .json(page)
                    .send()
                    .await?;
                let status = res.status();
                if !status.is_success() {
                    let detail = res.text().await.unwrap_or_default();
                    return Err(format!("SSR server answered {status}: {detail}").into());
                }
                Ok(res.json::<SsrResponse>().await?)
            })
        }
    }
}
