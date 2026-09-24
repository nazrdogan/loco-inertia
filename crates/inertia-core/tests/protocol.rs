use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    middleware::from_fn,
    response::{IntoResponse, Redirect, Response},
    routing::{get, put},
    Extension, Router,
};
use http_body_util::BodyExt;
use inertia_core::{inertia_middleware, Inertia, InertiaConfig, RootView};
use serde_json::{json, Value};
use tower::ServiceExt;

const VERSION: &str = "v1";

fn config() -> InertiaConfig {
    InertiaConfig::new(|view: &RootView<'_>| {
        Ok(format!(
            "<!doctype html><html><head><title>{}</title></head><body>{}</body></html>",
            view.page.component, view.body
        ))
    })
    .with_version(VERSION)
}

fn app() -> Router {
    Router::new()
        .route(
            "/users",
            get(|i: Inertia| async move {
                i.render("Users/Index", json!({ "users": ["ann", "bob"] }))
                    .await
            }),
        )
        .route(
            "/xss",
            get(|i: Inertia| async move {
                i.render(
                    "Xss",
                    json!({ "html": "</script><script>alert(1)</script>" }),
                )
                .await
            }),
        )
        .route(
            "/users/1",
            put(|| async { Redirect::to("/users") })
                .patch(|| async { Redirect::to("/users") })
                .delete(|| async { Redirect::to("/users") })
                .post(|| async { Redirect::to("/users") }),
        )
        .layer(from_fn(inertia_middleware))
        .layer(Extension(config()))
}

async fn send(req: Request<Body>) -> Response {
    app().oneshot(req).await.unwrap()
}

async fn body_string(res: Response) -> String {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn get_req(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn inertia_req(method: Method, uri: &str, version: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Inertia", "true")
        .header("X-Inertia-Version", version)
        .body(Body::empty())
        .unwrap()
}

fn vary(res: &Response) -> Vec<String> {
    res.headers()
        .get_all(header::VARY)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect()
}

/// Extract and parse the JSON inside the data-page script.
fn embedded_page(html: &str) -> (String, Value) {
    let open = r#"<script data-page="app" type="application/json">"#;
    let start = html.find(open).expect("data-page script") + open.len();
    let end = start + html[start..].find("</script>").unwrap();
    let raw = html[start..end].to_string();
    let value = serde_json::from_str(&raw).unwrap();
    (raw, value)
}

#[tokio::test]
async fn first_visit_renders_html_with_mount_markup() {
    let res = send(get_req("/users?page=2")).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get("x-inertia").is_none());
    assert!(res.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert_eq!(vary(&res), ["X-Inertia"]);

    let html = body_string(res).await;
    let expected = r#"<script data-page="app" type="application/json">{"component":"Users/Index","props":{"errors":{},"users":["ann","bob"]},"url":"/users?page=2","version":"v1"}</script><div id="app"></div>"#;
    assert!(html.contains(expected), "{html}");
    assert!(html.contains("<title>Users/Index</title>"));
}

#[tokio::test]
async fn embedded_json_escapes_tags() {
    let html = body_string(send(get_req("/xss")).await).await;

    // Exactly one closing script tag: the one we emitted.
    assert_eq!(html.matches("</script>").count(), 1, "{html}");
    let (raw, page) = embedded_page(&html);
    assert!(!raw.contains('<') && !raw.contains('>'));
    assert!(raw.contains(concat!("\\", "u003c/script", "\\", "u003e")));
    assert_eq!(
        page["props"]["html"], "</script><script>alert(1)</script>",
        "escaped JSON must round-trip"
    );
}

#[tokio::test]
async fn inertia_request_returns_json_page() {
    let res = send(inertia_req(Method::GET, "/users?page=2", VERSION)).await;

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["x-inertia"], "true");
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(vary(&res), ["X-Inertia"]);

    let page: Value = serde_json::from_str(&body_string(res).await).unwrap();
    assert_eq!(
        page,
        json!({
            "component": "Users/Index",
            "props": { "errors": {}, "users": ["ann", "bob"] },
            "url": "/users?page=2",
            "version": "v1",
        })
    );
}

#[tokio::test]
async fn stale_version_on_get_returns_409_with_location() {
    let res = send(inertia_req(Method::GET, "/users?page=2", "old")).await;

    assert_eq!(res.status(), StatusCode::CONFLICT);
    assert_eq!(res.headers()["x-inertia-location"], "/users?page=2");
    assert_eq!(res.headers()["x-inertia-version"], VERSION);
    assert_eq!(vary(&res), ["X-Inertia"]);
}

#[tokio::test]
async fn stale_version_is_ignored_for_non_get_and_non_inertia_requests() {
    let res = send(inertia_req(Method::POST, "/users/1", "old")).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER); // axum's Redirect::to is 303

    let mut req = get_req("/users");
    req.headers_mut()
        .insert("x-inertia-version", "old".parse().unwrap());
    assert_eq!(send(req).await.status(), StatusCode::OK);
}

fn found(to: &'static str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, to)]).into_response()
}

fn redirect_app() -> Router {
    Router::new()
        .route(
            "/r",
            get(|| async { found("/users") })
                .post(|| async { found("/users") })
                .put(|| async { found("/users") })
                .patch(|| async { found("/users") })
                .delete(|| async { found("/users") }),
        )
        .layer(from_fn(inertia_middleware))
        .layer(Extension(config()))
}

#[tokio::test]
async fn found_becomes_see_other_for_inertia_put_patch_delete() {
    for method in [Method::PUT, Method::PATCH, Method::DELETE] {
        let res = redirect_app()
            .oneshot(inertia_req(method.clone(), "/r", VERSION))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER, "{method}");
        assert_eq!(res.headers()[header::LOCATION], "/users");
        assert_eq!(vary(&res), ["X-Inertia"]);
    }
}

#[tokio::test]
async fn found_is_kept_for_get_post_and_non_inertia_requests() {
    for method in [Method::GET, Method::POST] {
        let res = redirect_app()
            .oneshot(inertia_req(method.clone(), "/r", VERSION))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND, "{method}");
    }
    let req = Request::put("/r").body(Body::empty()).unwrap();
    let res = redirect_app().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
}

#[tokio::test]
async fn vary_is_appended_not_overwritten() {
    let app = Router::new()
        .route(
            "/",
            get(|| async { ([(header::VARY, "Accept-Encoding")], "ok") }),
        )
        .layer(from_fn(inertia_middleware))
        .layer(Extension(config()));
    let res = app.oneshot(get_req("/")).await.unwrap();
    assert_eq!(vary(&res), ["Accept-Encoding", "X-Inertia"]);
}

#[tokio::test]
async fn missing_version_matches_empty_client_version() {
    let app = Router::new()
        .route(
            "/",
            get(|i: Inertia| async move { i.render("Home", json!({})).await }),
        )
        .layer(from_fn(inertia_middleware))
        .layer(Extension(InertiaConfig::new(|v: &RootView<'_>| {
            Ok(v.body.to_string())
        })));
    let req = Request::get("/")
        .header("X-Inertia", "true")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let page: Value = serde_json::from_str(&body_string(res).await).unwrap();
    assert_eq!(page["version"], "");
}

#[tokio::test]
async fn custom_app_id_is_used_in_mount_markup() {
    let app = Router::new()
        .route(
            "/",
            get(|i: Inertia| async move { i.render("Home", json!({})).await }),
        )
        .layer(Extension(
            InertiaConfig::new(|v: &RootView<'_>| Ok(v.body.to_string())).with_app_id("root"),
        ));
    let html = body_string(app.oneshot(get_req("/")).await.unwrap()).await;
    assert!(html.starts_with(r#"<script data-page="root" type="application/json">"#));
    assert!(html.ends_with(r#"</script><div id="root"></div>"#));
}

#[tokio::test]
async fn history_flags_are_omitted_unless_true() {
    let mut page = inertia_core::Page::new("Home", json!({}), "/", "");
    page.encrypt_history = true;
    let v = serde_json::to_value(&page).unwrap();
    assert_eq!(v.get("clearHistory"), None);
    assert_eq!(v["encryptHistory"], true);
}

#[tokio::test]
async fn missing_config_extension_is_a_500() {
    let app = Router::new().route(
        "/",
        get(|i: Inertia| async move { i.render("Home", json!({})).await }),
    );
    let res = app.oneshot(get_req("/")).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
