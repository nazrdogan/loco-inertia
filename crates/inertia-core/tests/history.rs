#![cfg(feature = "cookie-flash")]
//! `encryptHistory` / `clearHistory`.

use axum::{
    body::Body,
    http::{header, Request},
    middleware::from_fn,
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Router,
};
use http_body_util::BodyExt;
use inertia_core::{
    cookie_flash_middleware, inertia_middleware, CookieFlash, CookieKey, Inertia, InertiaConfig,
    RootView,
};
use serde_json::{json, Value};
use tower::ServiceExt;

fn app(config: InertiaConfig) -> Router {
    Router::new()
        .route(
            "/",
            get(|i: Inertia| async move { i.render("Home", ()).await }),
        )
        .route(
            "/secret",
            get(|i: Inertia| async move { i.encrypt_history(true).render("Secret", ()).await }),
        )
        .route(
            "/public",
            get(|i: Inertia| async move { i.encrypt_history(false).render("Public", ()).await }),
        )
        .route(
            "/cleared",
            get(|i: Inertia| async move { i.clear_history().render("Home", ()).await }),
        )
        .route(
            "/logout",
            post(|i: Inertia| async move { i.redirect("/").clear_history().into_response() }),
        )
        .layer(from_fn(inertia_middleware))
        .layer(from_fn(cookie_flash_middleware))
        .layer(Extension(CookieFlash::new(CookieKey::from(&[3u8; 64]))))
        .layer(Extension(config))
}

fn config() -> InertiaConfig {
    InertiaConfig::new(|v: &RootView<'_>| Ok(v.body.to_string()))
}

async fn page(app: Router, req: Request<Body>) -> Value {
    let res = app.oneshot(req).await.unwrap();
    serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

fn inertia(uri: &str) -> axum::http::request::Builder {
    Request::get(uri).header("X-Inertia", "true")
}

#[tokio::test]
async fn history_flags_are_omitted_by_default() {
    let p = page(app(config()), inertia("/").body(Body::empty()).unwrap()).await;
    assert!(p.get("encryptHistory").is_none() && p.get("clearHistory").is_none());
}

#[tokio::test]
async fn encryption_can_be_enabled_globally_or_per_page() {
    let p = page(
        app(config()),
        inertia("/secret").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(p["encryptHistory"], true);

    let global = || app(config().with_encrypt_history(true));
    let p = page(global(), inertia("/").body(Body::empty()).unwrap()).await;
    assert_eq!(p["encryptHistory"], true);
    // A page can opt out of the global default.
    let p = page(global(), inertia("/public").body(Body::empty()).unwrap()).await;
    assert!(p.get("encryptHistory").is_none());
}

#[tokio::test]
async fn clear_history_on_the_current_page() {
    let p = page(
        app(config()),
        inertia("/cleared").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(p["clearHistory"], true);
}

fn cookie_of(res: &Response) -> String {
    res.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn clear_history_survives_a_redirect_once() {
    let res = app(config())
        .oneshot(
            Request::post("/logout")
                .header("X-Inertia", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 302);
    let cookie = cookie_of(&res);

    let res = app(config())
        .oneshot(
            inertia("/")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        cookie_of(&res),
        "inertia_flash=",
        "the flash cookie is consumed"
    );
    let p: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(p["clearHistory"], true);
    assert!(
        p.get("flash").is_none(),
        "clearHistory is not flash data: {p}"
    );
    assert_eq!(p["props"], json!({ "errors": {} }));
}
