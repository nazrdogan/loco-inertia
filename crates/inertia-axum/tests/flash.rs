#![cfg(feature = "cookie-flash")]
//! Shared props, validation errors, error bags and flash data, including the full
//! POST → redirect → GET round trip through the encrypted cookie store.

use std::convert::Infallible;

use axum::{
    body::Body,
    extract::Request,
    http::{header, StatusCode},
    middleware::{from_fn, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Router,
};
use http_body_util::BodyExt;
use inertia_axum::{
    cookie_flash_middleware, inertia_middleware, CookieFlash, CookieKey, FlashData, IncomingFlash,
    Inertia, InertiaConfig, OutgoingFlash, Prop, Props, RootView, SharedProps,
};
use serde_json::{json, Map, Value};
use tower::ServiceExt;

async fn share(mut req: Request, next: Next) -> Response {
    let shared = SharedProps::of(req.extensions_mut());
    shared.insert("app_name", "demo");
    shared.insert("title", "shared title");
    shared.insert(
        "user",
        Prop::lazy(|| async { Ok::<_, Infallible>(json!({ "id": 7 })) }),
    );
    next.run(req).await
}

fn routes() -> Router {
    Router::new()
        .route(
            "/form",
            get(|i: Inertia| async move { i.render("Form", json!({ "title": "Form" })).await }),
        )
        .route(
            "/form",
            post(|i: Inertia| async move {
                i.back()
                    .with_errors(json!({ "name": "The name is required." }))
                    .into_response()
            })
            .put(|i: Inertia| async move {
                i.redirect("/form")
                    .with_flash("message", "Saved!")
                    .into_response()
            }),
        )
        .route(
            "/two-checks",
            post(|i: Inertia| async move {
                i.back()
                    .with_errors(json!({ "name": "The name is required." }))
                    .with_errors(json!({ "email": "The email is taken." }))
                    .into_response()
            }),
        )
        .route(
            "/toast",
            get(|i: Inertia| async move { i.flash("toast", "now").render("Form", ()).await }),
        )
        .route(
            "/external",
            get(|i: Inertia| async move { i.location("https://example.com/pay") }),
        )
        .layer(from_fn(share))
}

fn config() -> InertiaConfig {
    InertiaConfig::new(|v: &RootView<'_>| Ok(v.body.to_string())).with_version("1")
}

/// Router without a store: incoming flash is injected by hand.
fn bare_app(incoming: Option<FlashData>) -> Router {
    routes()
        .layer(from_fn(inertia_middleware))
        .layer(from_fn(move |mut req: Request, next: Next| {
            let incoming = incoming.clone();
            async move {
                if let Some(data) = incoming {
                    req.extensions_mut().insert(IncomingFlash(data));
                }
                next.run(req).await
            }
        }))
        .layer(Extension(config()))
}

fn cookie_app() -> Router {
    routes()
        .layer(from_fn(inertia_middleware))
        .layer(from_fn(cookie_flash_middleware))
        .layer(Extension(CookieFlash::new(CookieKey::from(&[7u8; 64]))))
        .layer(Extension(config()))
}

fn inertia(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Inertia", "true")
        .header("X-Inertia-Version", "1")
}

async fn page(res: Response) -> Value {
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn set_cookie(res: &Response) -> Option<String> {
    res.headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_string())
}

/// `name=value` of a Set-Cookie header.
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_string()
}

#[tokio::test]
async fn shared_props_are_merged_under_page_props() {
    let res = bare_app(None)
        .oneshot(inertia("GET", "/form").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let page = page(res).await;
    assert_eq!(
        page["props"],
        json!({ "app_name": "demo", "title": "Form", "user": { "id": 7 }, "errors": {} })
    );
    // `title` was overridden by the page, so it is not reported as shared.
    assert_eq!(page["sharedProps"], json!(["app_name", "user"]));
}

#[tokio::test]
async fn errors_are_always_present_even_in_partial_reloads() {
    let req = inertia("GET", "/form")
        .header("X-Inertia-Partial-Component", "Form")
        .header("X-Inertia-Partial-Data", "title")
        .body(Body::empty())
        .unwrap();
    let page = page(bare_app(None).oneshot(req).await.unwrap()).await;
    assert_eq!(page["props"], json!({ "title": "Form", "errors": {} }));
}

#[tokio::test]
async fn incoming_errors_and_flash_are_rendered() {
    let incoming = FlashData {
        errors: Map::from_iter([("name".into(), json!("Required"))]),
        flash: Map::from_iter([("message".into(), json!("Hi"))]),
        clear_history: false,
    };
    let res = bare_app(Some(incoming))
        .oneshot(inertia("GET", "/form").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let page = page(res).await;
    assert_eq!(page["props"]["errors"], json!({ "name": "Required" }));
    assert_eq!(page["flash"], json!({ "message": "Hi" }));
}

#[tokio::test]
async fn flash_is_omitted_when_empty_and_can_be_set_on_render() {
    let res = bare_app(None)
        .oneshot(inertia("GET", "/form").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(page(res).await.get("flash").is_none());

    let res = bare_app(None)
        .oneshot(inertia("GET", "/toast").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(page(res).await["flash"], json!({ "toast": "now" }));
}

#[tokio::test]
async fn back_redirects_to_referer_with_errors() {
    let req = inertia("POST", "/form")
        .header("Host", "localhost")
        .header("Referer", "http://localhost/form?step=2")
        .body(Body::empty())
        .unwrap();
    let mut res = bare_app(None).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    // Same site: kept, as a relative URL.
    assert_eq!(res.headers()[header::LOCATION], "/form?step=2");
    let OutgoingFlash(data) = res.extensions_mut().remove::<OutgoingFlash>().unwrap();
    assert_eq!(
        Value::Object(data.errors),
        json!({ "name": "The name is required." })
    );

    let res = bare_app(None)
        .oneshot(inertia("POST", "/form").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.headers()[header::LOCATION], "/");
}

#[tokio::test]
async fn error_bag_nests_errors() {
    let req = inertia("POST", "/form")
        .header("X-Inertia-Error-Bag", "createUser")
        .body(Body::empty())
        .unwrap();
    let mut res = bare_app(None).oneshot(req).await.unwrap();
    let OutgoingFlash(data) = res.extensions_mut().remove::<OutgoingFlash>().unwrap();
    assert_eq!(
        Value::Object(data.errors),
        json!({ "createUser": { "name": "The name is required." } })
    );
}

#[tokio::test]
async fn repeated_errors_add_to_the_same_bag() {
    let req = inertia("POST", "/two-checks")
        .header("X-Inertia-Error-Bag", "createUser")
        .body(Body::empty())
        .unwrap();
    let mut res = bare_app(None).oneshot(req).await.unwrap();
    let OutgoingFlash(data) = res.extensions_mut().remove::<OutgoingFlash>().unwrap();
    assert_eq!(
        Value::Object(data.errors),
        json!({ "createUser": {
            "name": "The name is required.",
            "email": "The email is taken.",
        } })
    );
}

#[tokio::test]
async fn location_is_409_for_inertia_and_a_redirect_otherwise() {
    let res = bare_app(None)
        .oneshot(inertia("GET", "/external").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(
        res.headers()["x-inertia-location"],
        "https://example.com/pay"
    );

    let res = bare_app(None)
        .oneshot(Request::get("/external").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(res.headers()[header::LOCATION], "https://example.com/pay");
}

#[tokio::test]
async fn errors_survive_one_redirect_through_the_cookie() {
    // POST fails validation → redirect back, errors stored in the cookie.
    let res = cookie_app()
        .oneshot(
            inertia("POST", "/form")
                .header("Referer", "/form")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    let stored = set_cookie(&res).expect("flash cookie");
    assert!(stored.contains("HttpOnly") && stored.contains("SameSite=Lax"));
    assert!(
        !stored.contains("required"),
        "cookie must be encrypted: {stored}"
    );
    let cookie = cookie_pair(&stored);

    // The follow-up GET renders the errors and clears the cookie.
    let res = cookie_app()
        .oneshot(
            inertia("GET", "/form")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let cleared = set_cookie(&res).expect("removal cookie");
    assert!(
        cleared.starts_with("inertia_flash=;") && cleared.contains("Max-Age=0"),
        "{cleared}"
    );
    assert_eq!(
        page(res).await["props"]["errors"],
        json!({ "name": "The name is required." })
    );
}

#[tokio::test]
async fn put_redirect_with_flash_is_303_and_sets_the_cookie() {
    let res = cookie_app()
        .oneshot(inertia("PUT", "/form").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let cookie = cookie_pair(&set_cookie(&res).unwrap());

    let res = cookie_app()
        .oneshot(
            inertia("GET", "/form")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let page = page(res).await;
    assert_eq!(page["flash"], json!({ "message": "Saved!" }));
    assert_eq!(page["props"]["errors"], json!({}));
}

#[tokio::test]
async fn version_conflict_keeps_the_flash_for_the_reload() {
    let res = cookie_app()
        .oneshot(inertia("PUT", "/form").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let cookie = cookie_pair(&set_cookie(&res).unwrap());

    let res = cookie_app()
        .oneshot(
            Request::get("/form")
                .header("X-Inertia", "true")
                .header("X-Inertia-Version", "stale")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert!(
        set_cookie(&res).is_none(),
        "the cookie must survive the 409"
    );
}

#[tokio::test]
async fn forged_cookie_is_ignored() {
    let forged = format!("inertia_flash={}", r#"{"errors":{"name":"forged"}}"#);
    let res = cookie_app()
        .oneshot(
            inertia("GET", "/form")
                .header(header::COOKIE, forged)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page(res).await["props"]["errors"], json!({}));
}

#[tokio::test]
async fn nested_shared_props_follow_partial_rules() {
    let props = Props::new().with("a", 1);
    let shared = SharedProps::default();
    shared.insert("auth", Prop::always(json!({ "id": 1 })));
    let mut req = inertia("GET", "/x")
        .header("X-Inertia-Partial-Component", "X")
        .header("X-Inertia-Partial-Data", "a")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(shared);
    req.extensions_mut().insert(config());
    let (mut parts, _) = req.into_parts();
    let inertia =
        <Inertia as axum::extract::FromRequestParts<()>>::from_request_parts(&mut parts, &())
            .await
            .unwrap();
    let page = inertia.build_page("X", props).await.unwrap();
    assert_eq!(
        page.props,
        json!({ "a": 1, "auth": { "id": 1 }, "errors": {} })
    );
}
