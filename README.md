# loco-inertia

Inertia.js v3 server adapter for [Loco.rs](https://loco.rs) 1.x (axum).
Needs Rust 1.94+ (pinned in `rust-toolchain.toml`).

- `crates/inertia-axum` — the protocol for any axum app (no Loco); published separately.
- `crates/loco-inertia` — Loco `Initializer`, `settings.inertia` config, Tera root template.
- `examples/demo` — Loco app + Vite + React 19 + `@inertiajs/react` v3.

## Run the demo

```sh
cd examples/demo/frontend && npm install && npm run dev   # Vite on :5173
cd examples/demo && cargo run -- start                    # Loco on :5150
```

Open http://localhost:5150 and navigate with the links; the network tab shows
`/about` fetched as XHR with `X-Inertia: true` and a JSON page response.

Open http://localhost:5150/feed: `posts` is an infinite scroll prop (scroll down or up from
`/feed?page=5`; the "Even ids only" filter resets it), `stats` is deferred and `categories`
is optional ("Load categories"). http://localhost:5150/upload uploads files through `InertiaForm`
(an avatar and several photos, validated as images of at most 1 MB).

## Usage

```rust
use loco_inertia::{Inertia, Prop, Props};

async fn feed(inertia: Inertia) -> Result<Response> {
    let props = Props::new()
        .with("posts", Prop::value(ScrollData::new(posts))           // <InfiniteScroll data="posts">
            .scroll(ScrollMeta::numbered(page, last_page).match_on("id")))
        .with("stats", Prop::defer(|| async { load_stats().await }))  // fetched after render
        .with("categories", Prop::optional(|| async { load_categories().await }))  // on request
        .with("auth", Prop::always(user))  // survives partial reload filters
        .with("sidebar", Props::new().with("links", links));  // nested, `sidebar.links`
    Ok(inertia.render("Feed", props).await)
}
```

Forms go through `InertiaForm<T>` (a `validator`-checked JSON body). It answers the
client's live validation (Precognition, `useForm("post", url, data).validate(field)`) with
`204`/`422` without running the handler, and redirects back with the errors when the
submitted data is invalid; errors and flash messages survive the redirect in an encrypted
cookie:

```rust
async fn submit(inertia: Inertia, InertiaForm(form): InertiaForm<ContactForm>) -> Result<Response> {
    // Only valid input gets here.
    Ok(inertia.redirect("/contact").with_flash("message", "Thanks!").into_response())
}
```

`InertiaForm` reads JSON, `multipart/form-data` (forms with files) and urlencoded bodies.
Form fields are converted to the struct's types (`"42"` → `u32`, `"1"`/`"on"` → `bool`,
`""` → `None`, `tags[]` → `Vec`), and files are `UploadedFile`s:

```rust
#[derive(Deserialize, Validate)]
struct Profile { name: String, avatar: Option<UploadedFile>, photos: Vec<UploadedFile> }
```

`back()` only returns to pages on this site (same `Host`, or `allowed_redirect_hosts`, which
includes Loco's `server.host`), as a relative URL; anything else goes to `/`. Flash data
that would not fit in a cookie (~4 KB) is shrunk (first message per field, shorter
messages, no flash data, then the errors that fit plus `errors._overflow`) and logged.

Errors land in `props.errors` (under `props.errors.<bag>` when the form used `errorBag`),
flash data in `page.flash`. Share props with every page from any middleware via
`SharedProps::of(req.extensions_mut()).insert("key", value)`.

Any `Serialize` struct also works as props; see below for typed pages.

Other page-level controls:

- `Prop::….once()` / `.once_as("key")` / `.expires_in(ttl)` / `.fresh(true)`: resolved once,
  then the client reuses the value and the server skips it (`X-Inertia-Except-Once-Props`).
- `inertia.encrypt_history(true)` (or `settings.inertia.encrypt_history` for every page)
  keeps page data encrypted in `history.state`; `inertia.clear_history()` /
  `redirect(..).clear_history()` (e.g. on logout) rotates the key.

### CSRF

On by default: every response hands out a signed, JavaScript-readable `XSRF-TOKEN` cookie,
the Inertia client (and its Precognition client) echo it in `X-XSRF-TOKEN`, and unsafe
requests (POST/PUT/PATCH/DELETE) without a matching, validly signed token get `419`.
Exempt paths that authenticate differently, or turn it off:

```yaml
settings:
  inertia:
    csrf:
      exempt: ["/api/", "/webhooks/"]   # or: enabled: false
```

The signing key is `flash_secret`, so set it in production (the demo reads
`INERTIA_FLASH_SECRET`). Wire it in `Hooks::middlewares` with
`loco_inertia::InertiaLayer` (see `examples/demo/src/app.rs`).

## Status

- [x] Phase 1: protocol core (HTML/JSON, escaping, `Vary`, 409 on stale version, 302→303)
- [x] Lazy prop tree: optional / always / defer (groups) / merge / prepend / deep merge /
      match on, nested at any depth, partial reloads (`X-Inertia-Partial-Data` / `-Except` /
      `-Component`, `X-Inertia-Reset`) with dot paths
- [x] Shared props (`sharedProps`), `errors` always present, validation errors + error bags,
      `page.flash`, `back()` / `redirect()` / `location()`, encrypted-cookie flash store
      (`settings.inertia.flash_secret`, `secure_cookies`), `validator` integration
- [x] Vite: `vite()` Tera function (dev server with React Refresh / manifest with CSS and
      modulepreload), asset version from the manifest hash, production config serving
      `frontend/dist`
- [x] SSR: `SsrRenderer` + `HttpSsr` (Inertia Node server, `POST /render`), timeout and
      CSR fallback on any error, first visits only, `inertia_head` in Tera; demo `ssr.tsx`
      with hydration
- [x] TS types from Rust props via ts-rs (`InertiaPage`, `page` / `page_with`, typed shared
      props and flash), `cargo loco task inertia_generate` emitting typed controllers, `.tsx`
      pages and request tests
- [x] Loco 1.x (1.2)
- [x] `encryptHistory` / `clearHistory` (config default, per page, after a redirect)
- [x] CSRF: signed double-submit `XSRF-TOKEN` cookie, 419 on mismatch, exempt paths
- [x] Once props (`onceProps`, `X-Inertia-Except-Once-Props`, expiry, fresh)
- [x] Precognition: `InertiaForm<T>` (live validation, `Precognition-Validate-Only`,
      redirect back with errors), `Inertia::precognition_response`
- [x] Production hardening: flash cookie size limit, same-site `back()`, multipart /
      urlencoded forms with `UploadedFile`, SSR circuit breaker
- [x] Infinite scroll: `Prop::scroll` / `Props::scroll` with `ScrollMeta` (numbered or cursor
      pages, custom page name and wrapper, `match_on`), `scrollProps`, append/prepend by
      `X-Inertia-Infinite-Scroll-Merge-Intent`, reset via `X-Inertia-Reset`; `ScrollData<T>`
      with a TS type

## Typed props and generators

Page props are Rust structs with `#[derive(Serialize, TS)] #[ts(export)]` (ts-rs) and an
`InertiaPage` impl naming their component, so handlers write `inertia.page(HomeProps { … })`
and `cargo test` regenerates `frontend/src/types/*.ts` (`TS_RS_EXPORT_DIR` in
`.cargo/config.toml`). `frontend/src/inertia.d.ts` plugs the shared props and flash types
into Inertia, so `usePage().props` / `usePage().flash` are typed too. Lazy props go through
`inertia.page_with(props, |p| p.with("stats", Prop::defer(…)))`, declared in the struct as
`#[ts(optional)] Option<T>` fields.

```sh
cargo loco task inertia_generate name:posts actions:index,show,create
cargo test        # runs the generated request tests and exports the new props types
```

This writes `src/controllers/posts.rs` (typed props per GET action; `create` / `update` /
`delete` become POST / PUT / DELETE handlers that redirect back with a flash message),
`frontend/src/Pages/Posts/{Index,Show}.tsx` and `tests/posts_pages.rs`, and registers the
routes. Register the task in `Hooks::register_tasks` with
`tasks.register(loco_inertia::InertiaGenerate::new(Self::app_name()))`. (Loco 1.x's own
`generate controller` only emits JSON API controllers, so this is a separate task.)

## Production

```sh
cd examples/demo/frontend && npm run build          # dist/ (client + manifest) and ssr/ssr.js
cd examples/demo/frontend && npm run ssr            # SSR server on 127.0.0.1:13714
cd examples/demo && INERTIA_FLASH_SECRET=$(openssl rand -hex 48) cargo run -- start -e production
```

First visits are server-rendered through the SSR server (`settings.inertia.ssr`); the
client hydrates them. If the SSR server is down, slow or throws, the page renders
client-side and a warning is logged; after 3 failures in a row SSR is skipped for 30 s
(`ssr.failure_threshold`, `ssr.cooldown_secs`), so a hung SSR server does not add its
timeout to every first visit. Inertia (XHR) visits never touch it. The root
template takes the SSR head via `{{ inertia_head | safe }}`.

Loco only logs its own crates; add `inertia_axum` and `loco_inertia` to
`logger.override_filter` (see `examples/demo/config/*.yaml`) to see the adapter's logs.

The root template calls `{{ vite() | safe }}`: dev-server tags in development, the built
entry with its CSS and `modulepreload` links (from the manifest) in production, where Loco
serves `frontend/dist` under `/static`. The asset version is a hash of the manifest, so
after a deploy, open tabs get a 409 on their next visit and reload with the new bundle.
Set `INERTIA_SECURE_COOKIES=false` to test the production build over plain HTTP.

## Tests

```sh
cargo test --workspace --all-features   # unit, integration and form fuzz tests
scripts/check.sh                        # everything: fmt, clippy, tests, types, e2e
SKIP_E2E=1 scripts/check.sh             # the same without the browser tests
```

The end-to-end tests (`examples/demo/frontend/e2e/`) drive the real Inertia client in
Google Chrome against the demo in production mode (built assets, SSR). To run them before
every push, enable the versioned hook once per clone:

```sh
git config core.hooksPath .githooks     # `git push --no-verify` skips it
```
