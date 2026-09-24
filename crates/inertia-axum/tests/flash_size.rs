#![cfg(feature = "cookie-flash")]
//! Oversized flash data is shrunk to fit a cookie instead of being dropped by the browser.

use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    http::{header, Request},
    middleware::from_fn,
    response::IntoResponse,
    routing::{get, post},
    Extension, Router,
};
use http_body_util::BodyExt;
use inertia_axum::{
    cookie_flash_middleware, inertia_middleware, CookieFlash, CookieKey, Inertia, InertiaConfig,
    RootView, MAX_FLASH_COOKIE_BYTES, OVERFLOW_ERROR_KEY,
};
use serde_json::{json, Map, Value};
use tower::ServiceExt;

/// What the POST handler flashes: `(errors, flash)`.
type Payload = Arc<Mutex<(Value, Map<String, Value>)>>;

fn app(payload: Payload) -> Router {
    Router::new()
        .route(
            "/form",
            get(|i: Inertia| async move { i.render("Form", ()).await }),
        )
        .route(
            "/form",
            post(move |i: Inertia| {
                let (errors, flash) = payload.lock().unwrap().clone();
                async move {
                    let mut redirect = i.redirect("/form").with_errors(errors);
                    for (k, v) in flash {
                        redirect = redirect.with_flash(k, v);
                    }
                    redirect.into_response()
                }
            }),
        )
        .layer(from_fn(inertia_middleware))
        .layer(from_fn(cookie_flash_middleware))
        .layer(Extension(CookieFlash::new(CookieKey::from(&[4u8; 64]))))
        .layer(Extension(InertiaConfig::new(|v: &RootView<'_>| {
            Ok(v.body.to_string())
        })))
}

/// Submit, check the cookie size, and return the page the redirect leads to.
async fn round_trip(errors: Value, flash: Map<String, Value>) -> Value {
    let payload: Payload = Arc::new(Mutex::new((errors, flash)));
    let res = app(payload.clone())
        .oneshot(
            Request::post("/form")
                .header("X-Inertia", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let set_cookie = res.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        set_cookie.len() <= MAX_FLASH_COOKIE_BYTES,
        "cookie is {} bytes",
        set_cookie.len()
    );
    let cookie = set_cookie.split(';').next().unwrap().to_string();

    let res = app(payload)
        .oneshot(
            Request::get("/form")
                .header("X-Inertia", "true")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

fn fields(n: usize, value: impl Fn(usize) -> Value) -> Value {
    Value::Object(
        (0..n)
            .map(|i| (format!("field_{i:03}"), value(i)))
            .collect(),
    )
}

#[tokio::test]
async fn small_payloads_are_untouched() {
    let errors = json!({ "name": ["Required.", "Too short."] });
    let page = round_trip(
        errors.clone(),
        Map::from_iter([("message".into(), json!("Hi"))]),
    )
    .await;
    assert_eq!(page["props"]["errors"], errors);
    assert_eq!(page["flash"]["message"], "Hi");
}

#[tokio::test]
async fn many_messages_per_field_keep_the_first() {
    let errors = fields(20, |i| {
        json!([
            format!("First problem with field {i}, explained at length."),
            "x".repeat(60),
            "y".repeat(60)
        ])
    });
    let page = round_trip(errors, Map::new()).await;
    let errors = page["props"]["errors"].as_object().unwrap();
    assert_eq!(errors.len(), 20);
    assert_eq!(
        errors["field_007"],
        "First problem with field 7, explained at length."
    );
}

#[tokio::test]
async fn very_long_messages_are_shortened() {
    let errors = fields(5, |_| json!("z".repeat(900)));
    let page = round_trip(errors, Map::new()).await;
    let message = page["props"]["errors"]["field_000"].as_str().unwrap();
    assert_eq!(message.chars().count(), 201);
    assert!(message.ends_with('…'));
}

#[tokio::test]
async fn flash_data_goes_before_errors() {
    let flash: Map<String, Value> = (0..40)
        .map(|i| (format!("k{i}"), json!("f".repeat(150))))
        .collect();
    let page = round_trip(json!({ "email": "Invalid." }), flash).await;
    assert_eq!(page["props"]["errors"], json!({ "email": "Invalid." }));
    assert!(page.get("flash").is_none());
}

#[tokio::test]
async fn too_many_fields_keep_what_fits_and_say_so() {
    let errors = fields(300, |i| {
        json!(format!("Field {i} is invalid for a fairly long reason."))
    });
    let page = round_trip(errors, Map::new()).await;
    let errors = page["props"]["errors"].as_object().unwrap();
    assert!(errors.contains_key(OVERFLOW_ERROR_KEY));
    assert!(
        errors.len() > 10 && errors.len() < 300,
        "kept {}",
        errors.len()
    );
    assert_eq!(
        errors["field_000"],
        "Field 0 is invalid for a fairly long reason."
    );
}

#[tokio::test]
async fn error_bags_are_shrunk_too() {
    let bag = fields(20, |_| json!(["a".repeat(80), "b".repeat(80)]));
    let page = round_trip(json!({ "signup": bag }), Map::new()).await;
    assert_eq!(
        page["props"]["errors"]["signup"]["field_000"],
        "a".repeat(80)
    );
}
