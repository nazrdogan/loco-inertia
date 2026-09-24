# Changelog

Both crates (`inertia-axum`, `loco-inertia`) share one version.

## Unreleased

- Every public item is documented; `missing_docs` keeps it that way.
- `rust-version` declared and tested: 1.88 for `inertia-axum`, 1.94 for `loco-inertia`.
- `ScrollData`'s TypeScript type carries the field's doc comment.
- Demo: production config uses Loco's YAML-safe `<%= %>` template delimiters.

## 0.1.1 — 2026-09-24

- docs.rs builds `inertia-axum` with all features, so `Csrf`, `CookieFlash`, `HttpSsr` and
  the other feature-gated items are documented.

## 0.1.0 — 2026-09-24

First release.

- `inertia-axum`: the Inertia.js v3 protocol for axum — pages, partial reloads, optional /
  always / deferred / merge / once / infinite scroll props, shared props, validation errors
  and flash data (encrypted cookie), CSRF, Precognition, history encryption, SSR behind a
  circuit breaker.
- `loco-inertia`: Loco middleware and initializer, `settings.inertia`, Tera root template,
  Vite manifest integration, `InertiaForm` (JSON, urlencoded, multipart with files) and the
  `inertia_generate` scaffolding task.
