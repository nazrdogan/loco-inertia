//! [Loco](https://loco.rs) integration for [`inertia_core`].
//!
//! Add [`InertiaLayer`] to `Hooks::middlewares`, in front of Loco's default stack so it runs
//! inside Loco's request-id, logging and error handling layers:
//!
//! ```ignore
//! fn middlewares(ctx: &AppContext) -> Vec<Box<dyn MiddlewareLayer>> {
//!     let mut stack: Vec<Box<dyn MiddlewareLayer>> = vec![Box::new(InertiaLayer::new(ctx))];
//!     stack.extend(loco_rs::controller::middleware::default_middleware_stack(ctx));
//!     stack
//! }
//! ```
//!
//! ([`InertiaInitializer`] does the same from `Hooks::initializers`, but wraps the whole app,
//! outside Loco's middleware.) Configure it under `settings`:
//!
//! ```yaml
//! settings:
//!   inertia:
//!     version: "1"                              # optional asset version
//!     root_template: assets/views/inertia.html  # Tera template, rendered on first visits
//!     app_id: app                               # optional, defaults to "app"
//!     flash_secret: "…at least 64 bytes…"      # encrypts the flash cookie (errors, flash)
//!     secure_cookies: true                      # HTTPS-only flash cookie, for production
//!     vite: { entry: src/main.tsx, dev_server: http://localhost:5173 }  # see [`vite`]
//!     ssr:                                      # server-side render first visits
//!       url: http://127.0.0.1:13714             # the Inertia Node SSR server (default)
//!       timeout_ms: 2000                        # then fall back to client rendering
//!       failure_threshold: 3                    # failures in a row before SSR is skipped…
//!       cooldown_secs: 30                       # …for this long
//! ```
//!
//! Loco's logger only passes events from its own whitelist of crates. To see this adapter's
//! warnings and errors (SSR fallbacks, failing lazy props), add `inertia_core` and
//! `loco_inertia` via `logger.override_filter`, e.g.
//! `"loco_rs=info,tower_http=info,my_app=info,inertia_core=info,loco_inertia=info"`.
//!
//! `flash_secret` keys both the flash cookie and the CSRF token. Without it a random key is
//! generated at boot, so flash data and CSRF tokens do not survive restarts or work across
//! instances (visitors get 419s). In `production` it is required: boot fails without it.
//!
//! The root template receives `inertia` (the mount markup, render it with `{{ inertia | safe }}`)
//! and `page` (the page object, e.g. `{{ page.component }}`).

use std::{collections::BTreeMap, path::Path, sync::Arc};

use async_trait::async_trait;
use axum::{Extension, Router as AxumRouter};
use loco_rs::{
    app::{AppContext, Initializer},
    controller::middleware::MiddlewareLayer,
    Error, Result,
};
use serde::{Deserialize, Serialize};

mod form;
pub mod templates;
pub mod vite;
pub use form::{InertiaForm, UploadedFile};
pub use templates::InertiaGenerate;
pub use vite::{Vite, ViteSettings};

pub use inertia_core::{
    self, cookie_flash_middleware, csrf_middleware, inertia_middleware, CookieFlash, CookieKey,
    Csrf, HttpSsr, Inertia, InertiaConfig, InertiaPage, InertiaRedirect, IntoProp, IntoProps, Page,
    Prop, Props, RootView, ScrollData, ScrollMeta, SharedProps,
};

/// Default location of the root template, relative to the app's working directory.
pub const DEFAULT_ROOT_TEMPLATE: &str = "assets/views/inertia.html";

/// `settings.inertia` in the Loco config.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InertiaSettings {
    pub version: Option<String>,
    pub root_template: Option<String>,
    pub app_id: Option<String>,
    /// Key material for the flash cookie; at least 64 bytes.
    #[serde(skip_serializing)]
    pub flash_secret: Option<String>,
    #[serde(default)]
    pub secure_cookies: bool,
    /// Encrypt every page in the browser history (`encryptHistory`); pages can override it
    /// with `Inertia::encrypt_history`.
    #[serde(default)]
    pub encrypt_history: bool,
    /// CSRF protection (signed `XSRF-TOKEN` cookie + `X-XSRF-TOKEN` header), on unless
    /// `enabled: false`.
    pub csrf: Option<CsrfSettings>,
    /// Hosts whose `Referer` `back()` may return to besides the request's `Host`, for
    /// proxies that rewrite it. Loco's `server.host` (with and without `server.port`) is
    /// always included.
    #[serde(default)]
    pub allowed_redirect_hosts: Vec<String>,
    /// Asset tags and version from Vite; see [`vite`].
    pub vite: Option<ViteSettings>,
    /// Server-side rendering through the Inertia Node SSR server.
    pub ssr: Option<SsrSettings>,
    /// Refuse to start without `flash_secret` instead of using a random key. Set by
    /// [`InertiaSettings::from_context`] in the `production` environment.
    #[serde(skip)]
    pub require_flash_secret: bool,
}

/// `settings.inertia.csrf`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CsrfSettings {
    /// Defaults to `true`.
    pub enabled: Option<bool>,
    /// Path prefixes that skip the check, e.g. `/api/` or webhook endpoints.
    #[serde(default)]
    pub exempt: Vec<String>,
}

/// Everything [`apply`] installs: the adapter config plus the optional flash store and CSRF
/// protection. An `InertiaConfig` converts into a setup without either.
#[derive(Clone, Debug)]
pub struct Setup {
    pub config: InertiaConfig,
    pub flash: Option<CookieFlash>,
    pub csrf: Option<Csrf>,
}

impl From<InertiaConfig> for Setup {
    fn from(config: InertiaConfig) -> Self {
        Self {
            config,
            flash: None,
            csrf: None,
        }
    }
}

/// `settings.inertia.ssr`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SsrSettings {
    /// Defaults to `true` when the section is present.
    pub enabled: Option<bool>,
    /// Base URL of the SSR server; defaults to `http://127.0.0.1:13714`.
    pub url: Option<String>,
    /// Defaults to 2000.
    pub timeout_ms: Option<u64>,
    /// Consecutive failures before SSR is skipped for `cooldown_secs`; defaults to 3.
    pub failure_threshold: Option<u32>,
    /// Defaults to 30.
    pub cooldown_secs: Option<u64>,
}

impl SsrSettings {
    /// The renderer, or `None` when disabled.
    pub fn renderer(&self) -> Option<HttpSsr> {
        if self.enabled == Some(false) {
            return None;
        }
        let url = self.url.as_deref().unwrap_or(HttpSsr::DEFAULT_URL);
        Some(
            HttpSsr::new(url).with_timeout(std::time::Duration::from_millis(
                self.timeout_ms.unwrap_or(2000),
            )),
        )
    }
}

impl InertiaSettings {
    /// Read `settings.inertia` from a Loco app, allowing `back()` to return to the site's
    /// public host (`server.host`, with and without `server.port`).
    pub fn from_context(ctx: &AppContext) -> Result<Self> {
        let mut settings = Self::from_loco_settings(ctx.config.settings.as_ref())?;
        settings.require_flash_secret =
            ctx.environment == loco_rs::environment::Environment::Production;
        let server = &ctx.config.server;
        for url in [server.host.clone(), server.full_url()] {
            if let Some(authority) = url
                .parse::<axum::http::Uri>()
                .ok()
                .and_then(|u| u.authority().map(ToString::to_string))
            {
                if !settings.allowed_redirect_hosts.contains(&authority) {
                    settings.allowed_redirect_hosts.push(authority);
                }
            }
        }
        Ok(settings)
    }

    /// Read `settings.inertia`; a missing section yields the defaults.
    pub fn from_loco_settings(settings: Option<&serde_json::Value>) -> Result<Self> {
        match settings.and_then(|s| s.get("inertia")) {
            None | Some(serde_json::Value::Null) => Ok(Self::default()),
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| Error::Message(format!("invalid `settings.inertia`: {e}"))),
        }
    }

    /// The flash cookie store: keyed by `flash_secret`, or by a random per-process key.
    pub fn cookie_flash(&self) -> Result<CookieFlash> {
        Ok(CookieFlash::new(self.key()?).secure(self.secure_cookies))
    }

    /// CSRF protection from `csrf`, or `None` when disabled.
    pub fn csrf(&self, key: CookieKey) -> Option<Csrf> {
        let settings = self.csrf.clone().unwrap_or_default();
        if settings.enabled == Some(false) {
            return None;
        }
        Some(
            settings
                .exempt
                .into_iter()
                .fold(Csrf::new(key).secure(self.secure_cookies), Csrf::exempt),
        )
    }

    /// Config, flash store and CSRF protection, sharing one key.
    pub fn setup(self) -> Result<Setup> {
        let key = self.key()?;
        let flash = CookieFlash::new(key.clone()).secure(self.secure_cookies);
        let csrf = self.csrf(key);
        Ok(Setup {
            config: self.into_config()?,
            flash: Some(flash),
            csrf,
        })
    }

    /// The cookie key from `flash_secret`, or a random one.
    fn key(&self) -> Result<CookieKey> {
        let key = match &self.flash_secret {
            Some(secret) => CookieKey::try_from(secret.as_bytes()).map_err(|_| {
                Error::Message(
                    "`settings.inertia.flash_secret` must be at least 64 bytes".to_string(),
                )
            })?,
            None if self.require_flash_secret => {
                return Err(Error::Message(
                    "`settings.inertia.flash_secret` is required in production (64+ bytes); \
                     it keys the flash cookie and CSRF tokens"
                        .to_string(),
                ));
            }
            None => {
                tracing::warn!(
                    "`settings.inertia.flash_secret` is not set; using a random key, flash data \
                     and CSRF tokens will not survive restarts or work across instances"
                );
                CookieKey::generate()
            }
        };
        Ok(key)
    }

    /// Build the adapter config, loading the root template from disk.
    ///
    /// The version is `version` when set, else the Vite manifest hash (if any).
    pub fn into_config(self) -> Result<InertiaConfig> {
        let path = self
            .root_template
            .unwrap_or_else(|| DEFAULT_ROOT_TEMPLATE.to_string());
        let vite = self
            .vite
            .as_ref()
            .map(Vite::from_settings)
            .transpose()?
            .map(Arc::new);
        let version = self.version.or_else(|| {
            vite.as_ref()
                .and_then(|v| v.version().map(ToString::to_string))
        });
        let (mut tera, name) = load_root_template(&path)?;
        if let Some(vite) = &vite {
            vite.register(&mut tera);
        }
        let mut config = InertiaConfig {
            version,
            ..tera_root_template(Arc::new(tera), name)
        };
        config.encrypt_history = self.encrypt_history;
        config.allowed_redirect_hosts = self.allowed_redirect_hosts;
        if let Some(settings) = &self.ssr {
            if let Some(ssr) = settings.renderer() {
                config = config.with_ssr_breaker(
                    ssr,
                    settings
                        .failure_threshold
                        .unwrap_or(inertia_core::DEFAULT_SSR_FAILURE_THRESHOLD),
                    settings.cooldown_secs.map_or(
                        inertia_core::DEFAULT_SSR_COOLDOWN,
                        std::time::Duration::from_secs,
                    ),
                );
            }
        }
        if let Some(app_id) = self.app_id {
            config.app_id = app_id;
        }
        Ok(config)
    }
}

/// An [`InertiaConfig`] whose root template is the Tera template at `path`.
pub fn tera_root_template_file(path: impl AsRef<Path>) -> Result<InertiaConfig> {
    let (tera, name) = load_root_template(path.as_ref())?;
    Ok(tera_root_template(Arc::new(tera), name))
}

fn load_root_template(path: impl AsRef<Path>) -> Result<(tera::Tera, String)> {
    let path = path.as_ref();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("inertia.html")
        .to_string();
    let mut tera = tera::Tera::default();
    tera.add_template_file(path, Some(&name))
        .map_err(|e| Error::Message(format!("cannot load Inertia root template {path:?}: {e}")))?;
    Ok((tera, name))
}

/// An [`InertiaConfig`] rendering template `name` from an existing Tera instance.
///
/// Template context: `inertia` (mount markup) and `inertia_head` (SSR head tags), both
/// rendered with `| safe`, and `page` (the page object).
pub fn tera_root_template(tera: Arc<tera::Tera>, name: impl Into<String>) -> InertiaConfig {
    let name = name.into();
    InertiaConfig::new(move |view: &RootView<'_>| {
        let mut ctx = tera::Context::new();
        ctx.insert("inertia", view.body);
        ctx.insert("inertia_head", view.head);
        ctx.insert("page", view.page);
        tera.render(&name, &ctx).map_err(|e| {
            // Tera hides the cause (e.g. a failing `vite()` call) in the source chain.
            let mut msg = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(s) = source {
                msg = format!("{msg}: {s}");
                source = s.source();
            }
            msg.into()
        })
    })
}

/// Install the Inertia middleware, its config and, when set up, the flash cookie store and
/// CSRF protection on a router. Accepts an `InertiaConfig` or a full [`Setup`].
pub fn apply<S: Clone + Send + Sync + 'static>(
    router: axum::Router<S>,
    setup: impl Into<Setup>,
) -> axum::Router<S> {
    let Setup {
        config,
        flash,
        csrf,
    } = setup.into();
    // Innermost first: the handler sees the Inertia middleware, then the flash store, then
    // the CSRF check; the extensions wrap them all.
    let mut router = router.layer(axum::middleware::from_fn(inertia_middleware));
    if let Some(flash) = flash {
        router = router
            .layer(axum::middleware::from_fn(cookie_flash_middleware))
            .layer(Extension(flash));
    }
    if let Some(csrf) = csrf {
        router = router
            .layer(axum::middleware::from_fn(csrf_middleware))
            .layer(Extension(csrf));
    }
    router.layer(Extension(config))
}

/// Validation errors as the `errors` prop expects them: the first message per field.
///
/// ```ignore
/// if let Err(e) = form.validate() {
///     return Ok(inertia.back().with_errors(loco_inertia::validation_errors(&e)).into_response());
/// }
/// ```
pub fn validation_errors(errors: &validator::ValidationErrors) -> BTreeMap<String, String> {
    errors
        .field_errors()
        .into_iter()
        .filter_map(|(field, errs)| {
            let first = errs.first()?;
            let message = first.message.as_ref().map_or_else(
                || format!("The {field} field is invalid ({}).", first.code),
                ToString::to_string,
            );
            Some((field.to_string(), message))
        })
        .collect()
}

/// Loco initializer wiring Inertia into the app router.
#[derive(Default)]
pub struct InertiaInitializer {
    config: Option<InertiaConfig>,
}

impl InertiaInitializer {
    /// Configure from `settings.inertia` at boot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Use an explicit config and ignore `settings.inertia` (no flash store).
    pub fn with_config(config: InertiaConfig) -> Self {
        Self {
            config: Some(config),
        }
    }
}

#[async_trait]
impl Initializer for InertiaInitializer {
    fn name(&self) -> String {
        "inertia".to_string()
    }

    async fn after_routes(&self, router: AxumRouter, ctx: &AppContext) -> Result<AxumRouter> {
        let setup = match &self.config {
            Some(config) => Setup::from(config.clone()),
            None => InertiaSettings::from_context(ctx)?.setup()?,
        };
        Ok(apply(router, setup))
    }
}

/// Loco middleware layer wiring Inertia into the app router, inside Loco's middleware stack.
pub struct InertiaLayer {
    source: LayerSource,
}

enum LayerSource {
    Settings(std::result::Result<InertiaSettings, String>),
    Config(Setup),
}

impl InertiaLayer {
    /// Configure from `settings.inertia`. Invalid settings are reported when the router is built.
    pub fn new(ctx: &AppContext) -> Self {
        let settings = InertiaSettings::from_context(ctx).map_err(|e| e.to_string());
        Self {
            source: LayerSource::Settings(settings),
        }
    }

    /// Use an explicit config (or full [`Setup`]), ignoring `settings.inertia`.
    pub fn with_config(setup: impl Into<Setup>) -> Self {
        Self {
            source: LayerSource::Config(setup.into()),
        }
    }
}

impl MiddlewareLayer for InertiaLayer {
    fn name(&self) -> &'static str {
        "inertia"
    }

    fn config(&self) -> serde_json::Result<serde_json::Value> {
        match &self.source {
            LayerSource::Settings(Ok(settings)) => serde_json::to_value(settings),
            LayerSource::Settings(Err(e)) => Ok(serde_json::json!({ "error": e })),
            LayerSource::Config(setup) => Ok(serde_json::json!({
                "version": setup.config.version,
                "app_id": setup.config.app_id,
                "flash": setup.flash.is_some(),
                "csrf": setup.csrf.is_some(),
            })),
        }
    }

    fn apply(&self, app: AxumRouter<AppContext>) -> Result<AxumRouter<AppContext>> {
        let setup = match &self.source {
            LayerSource::Settings(Ok(settings)) => settings.clone().setup()?,
            LayerSource::Settings(Err(e)) => return Err(Error::Message(e.clone())),
            LayerSource::Config(setup) => setup.clone(),
        };
        Ok(apply(app, setup))
    }
}
