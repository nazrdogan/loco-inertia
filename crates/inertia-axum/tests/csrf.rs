#![cfg(feature = "csrf")]
//! Signed double-submit CSRF protection.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    middleware::from_fn,
    response::Response,
    routing::get,
    Extension, Router,
};
use axum_extra::extract::cookie::Key;
use inertia_axum::{csrf_middleware, Csrf};
use tower::ServiceExt;

fn app(key: Key, hits: Arc<AtomicUsize>) -> Router {
    let hit = move || {
        let hits = hits.clone();
        async move {
            hits.fetch_add(1, Ordering::SeqCst);
            "ok"
        }
    };
    Router::new()
        .route(
            "/",
            get(hit.clone())
                .post(hit.clone())
                .put(hit.clone())
                .delete(hit.clone()),
        )
        .route("/webhooks/github", axum::routing::post(hit))
        .layer(from_fn(csrf_middleware))
        .layer(Extension(Csrf::new(key).exempt("/webhooks/")))
}

fn key() -> Key {
    Key::from(&[9u8; 64])
}

async fn send(app: Router, req: Request<Body>) -> Response {
    app.oneshot(req).await.unwrap()
}

fn set_cookie(res: &Response) -> Option<String> {
    res.headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_string())
}

/// A token issued by the server: `(Cookie header, value to echo)`, where the echoed value
/// is what browsers send: the cookie read with `decodeURIComponent`.
async fn issued_token() -> (String, String) {
    let res = send(
        app(key(), Default::default()),
        Request::get("/").body(Body::empty()).unwrap(),
    )
    .await;
    let set = set_cookie(&res).expect("token cookie");
    let pair = set.split(';').next().unwrap().to_string();
    let raw = pair.split_once('=').unwrap().1;
    let echoed = decode_uri_component(raw);
    (pair, echoed)
}

/// JavaScript's `decodeURIComponent`, for the ASCII the token uses.
fn decode_uri_component(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            out.push(u8::from_str_radix(&hex, 16).unwrap() as char);
        } else {
            out.push(c);
        }
    }
    out
}

#[tokio::test]
async fn the_issued_cookie_is_percent_encoded_and_both_forms_are_accepted() {
    let res = send(
        app(key(), Default::default()),
        Request::get("/").body(Body::empty()).unwrap(),
    )
    .await;
    let set = set_cookie(&res).unwrap();
    let pair = set.split(';').next().unwrap().to_string();
    let raw = pair.split_once('=').unwrap().1.to_string();
    // The base64 MAC's `=` is sent as `%3D`, which browsers decode before echoing it.
    assert!(raw.contains("%3D"), "{raw}");
    for echoed in [decode_uri_component(&raw), raw.clone()] {
        let res = send(
            app(key(), Default::default()),
            post(Some(&pair), Some(&echoed)),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK, "{echoed}");
    }
}

fn post(cookie: Option<&str>, header_value: Option<&str>) -> Request<Body> {
    let mut req = Request::post("/");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    if let Some(h) = header_value {
        req = req.header("X-XSRF-TOKEN", h);
    }
    req.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn first_visit_gets_a_readable_signed_token_cookie() {
    let res = send(
        app(key(), Default::default()),
        Request::get("/").body(Body::empty()).unwrap(),
    )
    .await;
    let cookie = set_cookie(&res).unwrap();
    assert!(cookie.starts_with("XSRF-TOKEN="), "{cookie}");
    assert!(
        cookie.contains("SameSite=Lax") && cookie.contains("Path=/"),
        "{cookie}"
    );
    assert!(
        !cookie.contains("HttpOnly"),
        "the client must be able to read it: {cookie}"
    );
    // Signed: the MAC is part of the value, the 64-hex-char token follows it.
    let value = cookie.split(';').next().unwrap().split_once('=').unwrap().1;
    assert!(value.len() > 64, "{value}");
}

#[tokio::test]
async fn a_valid_token_is_not_reissued() {
    let (cookie, _) = issued_token().await;
    let req = Request::get("/")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    assert!(set_cookie(&send(app(key(), Default::default()), req).await).is_none());
}

#[tokio::test]
async fn unsafe_requests_need_the_matching_header() {
    let (cookie, token) = issued_token().await;
    for method in [Method::POST, Method::PUT, Method::DELETE] {
        let hits = Arc::new(AtomicUsize::new(0));
        let req = Request::builder()
            .method(method.clone())
            .uri("/")
            .header(header::COOKIE, &cookie)
            .header("X-XSRF-TOKEN", &token)
            .body(Body::empty())
            .unwrap();
        let res = send(app(key(), hits.clone()), req).await;
        assert_eq!(res.status(), StatusCode::OK, "{method}");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn missing_or_wrong_tokens_are_rejected_before_the_handler() {
    let (cookie, token) = issued_token().await;
    let cases = [
        post(None, None),
        post(Some(&cookie), None),
        post(None, Some(&token)),
        post(Some(&cookie), Some("nope")),
        post(Some(&cookie), Some(&format!("{token}x"))),
    ];
    for req in cases {
        let hits = Arc::new(AtomicUsize::new(0));
        let res = send(app(key(), hits.clone()), req).await;
        assert_eq!(res.status().as_u16(), 419);
        assert_eq!(hits.load(Ordering::SeqCst), 0, "handler must not run");
    }
}

#[tokio::test]
async fn forged_or_foreign_cookies_fail_the_signature_check() {
    // An attacker who can plant cookies (e.g. from a sibling subdomain) picks both values.
    let forged = "a".repeat(64);
    let res = send(
        app(key(), Default::default()),
        post(Some(&format!("XSRF-TOKEN={forged}")), Some(&forged)),
    )
    .await;
    assert_eq!(res.status().as_u16(), 419);

    // A token signed with another app's key.
    let (cookie, token) = issued_token().await;
    let res = send(
        app(Key::from(&[1u8; 64]), Default::default()),
        post(Some(&cookie), Some(&token)),
    )
    .await;
    assert_eq!(res.status().as_u16(), 419);
}

#[tokio::test]
async fn exempt_paths_and_safe_methods_skip_the_check() {
    let res = send(
        app(key(), Default::default()),
        Request::post("/webhooks/github")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    for method in [Method::HEAD, Method::OPTIONS] {
        let req = Request::builder()
            .method(method.clone())
            .uri("/")
            .body(Body::empty())
            .unwrap();
        let res = send(app(key(), Default::default()), req).await;
        assert_ne!(res.status().as_u16(), 419, "{method}");
    }
}

#[tokio::test]
async fn without_the_extension_nothing_is_checked() {
    let app = Router::new()
        .route("/", axum::routing::post(|| async { "ok" }))
        .layer(from_fn(csrf_middleware));
    let res = send(app, post(None, None)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(set_cookie(&res).is_none());
}

#[tokio::test]
async fn every_token_cookie_is_verified_on_its_own() {
    // A planted, unsigned cookie sent before the real one (e.g. with a more specific path)
    // must not pass because the real one next to it is validly signed.
    let (cookie, token) = issued_token().await;
    let forged = "a".repeat(64);
    let both = format!("XSRF-TOKEN={forged}; {cookie}");

    let res = send(
        app(key(), Default::default()),
        post(Some(&both), Some(&forged)),
    )
    .await;
    assert_eq!(res.status().as_u16(), 419);

    let res = send(
        app(key(), Default::default()),
        post(Some(&both), Some(&token)),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn exemptions_match_whole_path_segments() {
    let app = Router::new()
        .route("/api", axum::routing::post(|| async { "ok" }))
        .route("/api/hook", axum::routing::post(|| async { "ok" }))
        .route("/apiary", axum::routing::post(|| async { "ok" }))
        .layer(from_fn(csrf_middleware))
        .layer(Extension(Csrf::new(key()).exempt("/api")));
    for (path, exempt) in [("/api", true), ("/api/hook", true), ("/apiary", false)] {
        let res = send(
            app.clone(),
            Request::post(path).body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(res.status() == StatusCode::OK, exempt, "{path}");
    }
}
