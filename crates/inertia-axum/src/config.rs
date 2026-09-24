use std::{fmt, sync::Arc};

use crate::{CircuitBreaker, Page, SsrRenderer};

/// Error type of fallible callbacks: the root template, lazy props, SSR.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// What the root template receives on a first (non-Inertia) visit.
#[derive(Debug)]
pub struct RootView<'a> {
    /// The page being rendered.
    pub page: &'a Page,
    /// The mount markup (`<script data-page=…>…</script><div id=…></div>`), server-rendered
    /// when SSR succeeded; insert it verbatim, without HTML-escaping, inside `<body>`.
    pub body: &'a str,
    /// Server-rendered `<head>` tags, empty without SSR; insert verbatim inside `<head>`.
    pub head: &'a str,
}

/// Renders the full HTML document for a first visit.
pub type RootTemplate = Arc<dyn Fn(&RootView<'_>) -> Result<String, BoxError> + Send + Sync>;

/// Adapter configuration, made available to handlers and the middleware via
/// `axum::Extension<InertiaConfig>`.
#[derive(Clone)]
pub struct InertiaConfig {
    /// Current asset version. Inertia visits from clients with another one get a `409` and
    /// reload the page; `None` counts as `""`.
    pub version: Option<String>,
    /// Renders the HTML document of first visits.
    pub root_template: RootTemplate,
    /// Id of the root element and `data-page` value of the page script (default `app`).
    pub app_id: String,
    /// Server-side renderer for first visits; `None` renders client-side only.
    pub ssr: Option<Arc<dyn SsrRenderer>>,
    /// Encrypt the page data the client keeps in `history.state` for every page (the key
    /// lives in the browser session; `clearHistory` rotates it). Per request:
    /// `Inertia::encrypt_history`.
    pub encrypt_history: bool,
    /// Hosts (`example.com`, `example.com:8443`) besides the request's `Host` whose
    /// `Referer` `Inertia::back` may redirect to, for proxies that rewrite `Host`.
    pub allowed_redirect_hosts: Vec<String>,
}

impl InertiaConfig {
    /// A config rendering first visits with `root_template`: no version, app id `app`,
    /// no SSR.
    pub fn new<F>(root_template: F) -> Self
    where
        F: Fn(&RootView<'_>) -> Result<String, BoxError> + Send + Sync + 'static,
    {
        Self {
            version: None,
            root_template: Arc::new(root_template),
            app_id: "app".to_string(),
            ssr: None,
            encrypt_history: false,
            allowed_redirect_hosts: Vec::new(),
        }
    }

    #[must_use]
    /// Encrypt every page in the browser history unless a handler says otherwise.
    pub fn with_encrypt_history(mut self, encrypt: bool) -> Self {
        self.encrypt_history = encrypt;
        self
    }

    /// Server-side render first visits with `ssr`, behind a [`CircuitBreaker`] (3 failures
    /// in a row → client-side rendering for 30 s).
    #[must_use]
    pub fn with_ssr(self, ssr: impl SsrRenderer + 'static) -> Self {
        self.with_ssr_breaker(
            ssr,
            crate::DEFAULT_SSR_FAILURE_THRESHOLD,
            crate::DEFAULT_SSR_COOLDOWN,
        )
    }

    /// Like [`InertiaConfig::with_ssr`] with a custom circuit breaker.
    #[must_use]
    pub fn with_ssr_breaker(
        mut self,
        ssr: impl SsrRenderer + 'static,
        threshold: u32,
        cooldown: std::time::Duration,
    ) -> Self {
        self.ssr = Some(Arc::new(CircuitBreaker::new(ssr, threshold, cooldown)));
        self
    }

    #[must_use]
    /// Set the asset version, e.g. a hash of the build manifest.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    #[must_use]
    /// Use another root element id than `app`.
    pub fn with_app_id(mut self, app_id: impl Into<String>) -> Self {
        self.app_id = app_id.into();
        self
    }

    /// The version as sent to and compared against the client (`""` when unset).
    pub fn version_str(&self) -> &str {
        self.version.as_deref().unwrap_or("")
    }
}

impl fmt::Debug for InertiaConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InertiaConfig")
            .field("version", &self.version)
            .field("app_id", &self.app_id)
            .field("ssr", &self.ssr.is_some())
            .field("encrypt_history", &self.encrypt_history)
            .finish_non_exhaustive()
    }
}
