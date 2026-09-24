Build a Rust Inertia.js v3 adapter for Loco.rs (axum). Start with the smallest end-to-end working slice, not the full feature set.

## Structure
Cargo workspace:
- crates/inertia-core   — protocol only, depends on axum/serde. NO loco dependency.
- crates/loco-inertia   — Loco Initializer, config from Loco settings, Tera helpers.
- examples/demo         — `loco new` app + Vite + React 19 + @inertiajs/react v3.

## Phase 1 (do this now, stop when it works)
In inertia-core:
1. `Page { component, props, url, version }` (serde). Omit `clearHistory`/`encryptHistory` unless true.
2. `InertiaConfig { version: Option<String>, root template fn, app_id = "app" }`, provided via `Extension`.
3. `Inertia` extractor (FromRequestParts) with `render(component, props: impl Serialize) -> Response`:
   - Request has `X-Inertia` header → JSON page, response header `X-Inertia: true`.
   - Otherwise → HTML via root template, mount markup exactly:
     `<script data-page="app" type="application/json">{json}</script><div id="app"></div>`
     JSON must escape `<` and `>` as `\u003c` / `\u003e` (like PHP JSON_HEX_TAG).
   - `url` = path + query of the request.
4. `inertia_middleware` (axum::middleware::from_fn):
   - Always set `Vary: X-Inertia`.
   - Inertia GET with `X-Inertia-Version` != server version → 409 + `X-Inertia-Location: <request url>`.
   - Inertia request, response 302, method PUT/PATCH/DELETE → 303.
5. Integration tests with `tower::ServiceExt::oneshot` for: first visit HTML, escaped JSON (props containing `</script>`), Inertia JSON response, 409 on stale version, 302→303.

In examples/demo:
- Two pages (Home, About) rendered from Loco controllers with props, linked via `<Link>`. Verify in browser that navigation returns JSON (network tab).
- Vite dev server for assets; hardcode the script tags in the root template for now.

## Later phases (do NOT implement yet, but keep the design open for them)
- Lazy prop tree instead of serde_json::Value: optional / always / defer / merge props, nested at any depth, partial reloads via X-Inertia-Partial-Data / -Except / -Component with dot-notation paths.
- Shared props, flash, validation errors via session + error bags (X-Inertia-Error-Bag).
- Vite manifest + asset version from manifest hash.
- SSR via Node sidecar: POST page JSON to http://127.0.0.1:13714/render, response {head: string[], body: string}, fall back to CSR on any error, only on non-Inertia requests.
- TS type generation from Rust props structs (ts-rs or specta), `cargo loco generate` scaffolds with .tsx pages.

## References (read before coding)
- https://inertiajs.com/docs/v3 — "The Protocol" page
- github.com/inertiajs/inertia-laravel (3.x branch): src/Response.php, src/Middleware.php, src/Directive.php, tests/
- github.com/mstallmo/inertia-loco — existing v1/v2-era Loco adapter; borrow the Loco config/extractor ideas, not the props model.

## Rules
- Keep inertia-core framework-agnostic from Loco.
- Every protocol behavior gets a test.
- Stop after Phase 1 and summarize what works and what's next.
