#![cfg(feature = "ssr")]
//! Server-side rendering through a fake Inertia SSR server.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
    Json, Router,
};
use http_body_util::BodyExt;
use inertia_axum::{
    HttpSsr, Inertia, InertiaConfig, Page, RootView, SsrFuture, SsrRenderer, SsrResponse,
};
use serde_json::{json, Value};
use tower::ServiceExt;

#[derive(Clone, Copy)]
enum Behaviour {
    Ok,
    Fail,
    Slow,
}

/// Start a fake SSR server; returns its base URL and the pages it received.
async fn ssr_server(behaviour: Behaviour) -> (String, Arc<Mutex<Vec<Value>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let app = Router::new().route(
        "/render",
        post(move |Json(page): Json<Value>| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(page.clone());
                match behaviour {
                    Behaviour::Fail => {
                        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "window is not defined" })));
                    }
                    Behaviour::Slow => tokio::time::sleep(Duration::from_secs(2)).await,
                    Behaviour::Ok => {}
                }
                (
                    StatusCode::OK,
                    Json(json!({
                        "head": ["<title>SSR title</title>", "<meta name=\"x\" content=\"y\">"],
                        "body": format!(
                            "<script data-page=\"app\" type=\"application/json\">{}</script><div data-server-rendered=\"true\" id=\"app\"><h1>{}</h1></div>",
                            page, page["props"]["greeting"].as_str().unwrap_or_default()
                        ),
                    })),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, seen)
}

fn root(view: &RootView<'_>) -> Result<String, inertia_axum::BoxError> {
    Ok(format!(
        "<head>{}</head><body>{}</body>",
        view.head, view.body
    ))
}

fn app(config: InertiaConfig) -> Router {
    Router::new()
        .route(
            "/",
            get(|i: Inertia| async move { i.render("Home", json!({ "greeting": "Hello SSR" })).await }),
        )
        .layer(axum::Extension(config))
}

async fn get_html(app: Router, inertia: bool) -> (StatusCode, String) {
    let mut req = Request::get("/?q=1");
    if inertia {
        req = req.header("X-Inertia", "true");
    }
    let res = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

const CSR_BODY: &str = r#"<body><script data-page="app" type="application/json">{"component":"Home","props":{"errors":{},"greeting":"Hello SSR"},"url":"/?q=1","version":""}</script><div id="app"></div></body>"#;

#[tokio::test]
async fn first_visit_uses_ssr_head_and_body() {
    let (url, seen) = ssr_server(Behaviour::Ok).await;
    let config = InertiaConfig::new(root).with_ssr(HttpSsr::new(url));
    let (status, html) = get_html(app(config), false).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        html.starts_with("<head><title>SSR title</title>\n<meta name=\"x\" content=\"y\"></head>"),
        "{html}"
    );
    assert!(
        html.contains(
            r#"<div data-server-rendered="true" id="app"><h1>Hello SSR</h1></div></body>"#
        ),
        "{html}"
    );
    // The SSR server received the full page object.
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [
            json!({ "component": "Home", "props": { "errors": {}, "greeting": "Hello SSR" }, "url": "/?q=1", "version": "" })
        ]
    );
}

#[tokio::test]
async fn inertia_requests_never_hit_the_ssr_server() {
    let (url, seen) = ssr_server(Behaviour::Ok).await;
    let config = InertiaConfig::new(root).with_ssr(HttpSsr::new(url));
    let (status, body) = get_html(app(config), true).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["component"],
        "Home"
    );
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ssr_error_falls_back_to_client_rendering() {
    let (url, seen) = ssr_server(Behaviour::Fail).await;
    let config = InertiaConfig::new(root).with_ssr(HttpSsr::new(url));
    let (status, html) = get_html(app(config), false).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(html, format!("<head></head>{CSR_BODY}"));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn unreachable_ssr_server_falls_back_to_client_rendering() {
    // Bind and drop a listener to get a port nobody listens on.
    let port = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config =
        InertiaConfig::new(root).with_ssr(HttpSsr::new(format!("http://127.0.0.1:{port}")));
    let (status, html) = get_html(app(config), false).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(html, format!("<head></head>{CSR_BODY}"));
}

#[tokio::test]
async fn slow_ssr_server_times_out_to_client_rendering() {
    let (url, _) = ssr_server(Behaviour::Slow).await;
    let ssr = HttpSsr::new(url).with_timeout(Duration::from_millis(200));
    let started = std::time::Instant::now();
    let (_, html) = get_html(app(InertiaConfig::new(root).with_ssr(ssr)), false).await;

    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(html, format!("<head></head>{CSR_BODY}"));
}

/// Any renderer can be plugged in, e.g. one embedding a JS runtime.
struct Static;

impl SsrRenderer for Static {
    fn render<'a>(&'a self, page: &'a Page) -> SsrFuture<'a> {
        Box::pin(async move {
            Ok(SsrResponse {
                head: vec![],
                body: format!("<div>{}</div>", page.component),
            })
        })
    }
}

#[tokio::test]
async fn custom_renderer() {
    let (_, html) = get_html(app(InertiaConfig::new(root).with_ssr(Static)), false).await;
    assert_eq!(html, "<head></head><body><div>Home</div></body>");
}
