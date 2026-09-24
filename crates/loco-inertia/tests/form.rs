//! `InertiaForm`: precognition, redirect-back with errors, valid input.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use http_body_util::BodyExt;
use loco_inertia::{apply, CookieFlash, CookieKey, Inertia, InertiaConfig, InertiaForm, Setup};
use serde::Deserialize;
use serde_json::{json, Value};
use tower::ServiceExt;

#[derive(Debug, Deserialize, validator::Validate)]
struct Signup {
    #[validate(length(min = 2, message = "Name is too short."))]
    name: String,
    #[validate(email(message = "Invalid email."))]
    email: String,
}

fn app(saved: Arc<AtomicUsize>) -> Router {
    let router = Router::new().route(
        "/signup",
        post(move |i: Inertia, InertiaForm(form): InertiaForm<Signup>| {
            let saved = saved.clone();
            async move {
                saved.fetch_add(1, Ordering::SeqCst);
                i.redirect("/welcome")
                    .with_flash("message", format!("Hi {}", form.name))
                    .into_response()
            }
        }),
    );
    // Flash store, no CSRF: this test is about the form.
    apply(
        router,
        Setup {
            config: InertiaConfig::new(|v: &loco_inertia::RootView<'_>| Ok(v.body.to_string())),
            flash: Some(CookieFlash::new(CookieKey::from(&[5u8; 64]))),
            csrf: None,
        },
    )
}

async fn submit(body: Value, headers: &[(&str, &str)]) -> (Response, usize, Value) {
    let saved = Arc::new(AtomicUsize::new(0));
    let mut req = Request::post("/signup")
        .header(header::CONTENT_TYPE, "application/json")
        .header("X-Inertia", "true")
        .header("Referer", "/signup");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut res = app(saved.clone())
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let bytes = std::mem::take(res.body_mut())
        .collect()
        .await
        .unwrap()
        .to_bytes();
    (
        res,
        saved.load(Ordering::SeqCst),
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn precognition_validates_without_running_the_handler() {
    let bad = json!({ "name": "A", "email": "nope" });
    let (res, saved, body) = submit(
        bad,
        &[
            ("Precognition", "true"),
            ("Precognition-Validate-Only", "email"),
        ],
    )
    .await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.headers()["precognition"], "true");
    assert_eq!(body["errors"], json!({ "email": "Invalid email." }));
    assert_eq!(
        saved, 0,
        "the action must not run on a precognition request"
    );

    // Valid data: 204, and still no action.
    let good = json!({ "name": "Ann", "email": "ann@example.com" });
    let (res, saved, _) = submit(good, &[("Precognition", "true")]).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(res.headers()["precognition-success"], "true");
    assert_eq!(saved, 0);
}

#[tokio::test]
async fn invalid_input_redirects_back_with_errors() {
    let (res, saved, _) = submit(
        json!({ "name": "A", "email": "nope" }),
        &[("X-Inertia-Error-Bag", "signup")],
    )
    .await;
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(res.headers()[header::LOCATION], "/signup");
    assert!(res.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .starts_with("inertia_flash="));
    assert_eq!(saved, 0);
}

#[tokio::test]
async fn valid_input_reaches_the_handler() {
    let (res, saved, _) = submit(json!({ "name": "Ann", "email": "ann@example.com" }), &[]).await;
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(res.headers()[header::LOCATION], "/welcome");
    assert_eq!(saved, 1);
}

#[tokio::test]
async fn malformed_json_is_rejected() {
    let saved = Arc::new(AtomicUsize::new(0));
    let req = Request::post("/signup")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{nope"))
        .unwrap();
    let res = app(saved.clone()).oneshot(req).await.unwrap();
    assert!(res.status().is_client_error());
    assert_eq!(saved.load(Ordering::SeqCst), 0);
}
