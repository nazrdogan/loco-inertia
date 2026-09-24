# inertia-axum

Server side of the [Inertia.js v3 protocol](https://inertiajs.com/docs/v3) for
[axum](https://github.com/tokio-rs/axum), with no framework on top. Using
[Loco](https://loco.rs)? Take [`loco-inertia`](https://crates.io/crates/loco-inertia), which
builds on this crate and adds settings, Tera/Vite root templates, validated forms and
scaffolding.

```rust
use axum::{routing::get, Extension, Router};
use inertia_axum::{inertia_middleware, Inertia, InertiaConfig};

async fn home(inertia: Inertia) -> axum::response::Response {
    inertia.render("Home", serde_json::json!({ "greeting": "hi" })).await
}

let config = InertiaConfig::new(|view: &inertia_axum::RootView<'_>| {
    Ok(format!("<!doctype html><html><body>{}</body></html>", view.body))
});
let app: Router = Router::new()
    .route("/", get(home))
    .layer(axum::middleware::from_fn(inertia_middleware))
    .layer(Extension(config));
```

- Page rendering: JSON for Inertia visits, the root HTML (page data escaped for `<script>`)
  on first visits; asset version checks (409) and 302 → 303 redirects.
- Props: optional, always, deferred (with groups), merge / prepend / deep merge, match-on,
  once props and infinite scroll, nested at any depth, with partial reloads by dot path.
- Shared props, validation errors with error bags, flash data, `back()` limited to this site.
- Precognition (live validation) responses and history encryption.
- Features: `cookie-flash` (encrypted flash cookie), `csrf` (signed `XSRF-TOKEN`), `ssr`
  (HTTP client for the Inertia Node SSR server, behind a circuit breaker), `ts`
  (`ts-rs` types for wire structs).

License: MIT.
