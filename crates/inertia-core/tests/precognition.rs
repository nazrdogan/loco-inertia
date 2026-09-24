//! Precognition (live validation) responses.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Extension, Router,
};
use http_body_util::BodyExt;
use inertia_core::{Inertia, InertiaConfig, RootView};
use serde_json::{json, Value};
use tower::ServiceExt;

fn app() -> Router {
    Router::new()
        .route(
            "/users",
            post(|i: Inertia| async move {
                // As if validating the submitted data found these problems:
                let errors = json!({
                    "name": "The name is required.",
                    "email": ["Invalid email.", "Already taken."],
                    "address.street": "Required.",
                });
                if i.is_precognitive() {
                    return i.precognition_response(errors);
                }
                "saved".into_response()
            }),
        )
        .layer(Extension(InertiaConfig::new(|v: &RootView<'_>| {
            Ok(v.body.to_string())
        })))
}

async fn send(headers: &[(&str, &str)]) -> (Response, Value) {
    let mut req = Request::post("/users");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut res = app()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = std::mem::take(res.body_mut())
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (res, body)
}

#[tokio::test]
async fn errors_come_back_as_422_limited_to_the_validated_fields() {
    let (res, body) = send(&[
        ("Precognition", "true"),
        ("Precognition-Validate-Only", "email"),
    ])
    .await;

    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.headers()["precognition"], "true");
    assert_eq!(res.headers()["vary"], "Precognition");
    assert_eq!(
        body["errors"],
        json!({ "email": ["Invalid email.", "Already taken."] })
    );
}

#[tokio::test]
async fn valid_fields_get_204_with_the_success_header() {
    // `phone` has no error even though other fields do.
    let (res, _) = send(&[
        ("Precognition", "true"),
        ("Precognition-Validate-Only", "phone"),
    ])
    .await;

    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(res.headers()["precognition"], "true");
    assert_eq!(res.headers()["precognition-success"], "true");
}

#[tokio::test]
async fn without_validate_only_every_error_is_returned() {
    let (res, body) = send(&[("Precognition", "true")]).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["errors"].as_object().unwrap().len(), 3);
}

#[tokio::test]
async fn nested_fields_match_their_parent() {
    let (_, body) = send(&[
        ("Precognition", "true"),
        ("Precognition-Validate-Only", "address"),
    ])
    .await;
    assert_eq!(body["errors"], json!({ "address.street": "Required." }));

    // A prefix that is not a path segment does not match.
    let (res, _) = send(&[
        ("Precognition", "true"),
        ("Precognition-Validate-Only", "addr"),
    ])
    .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn ordinary_requests_are_not_precognitive() {
    let (res, _) = send(&[("Precognition", "false")]).await;
    assert_eq!(res.status(), StatusCode::OK);
    let (res, _) = send(&[]).await;
    assert_eq!(res.status(), StatusCode::OK);
}
