//! `Inertia::back` only follows referers on this site.

use axum::{
    body::Body,
    http::{header, Request},
    response::IntoResponse,
    routing::post,
    Extension, Router,
};
use inertia_core::{Inertia, InertiaConfig, RootView};
use tower::ServiceExt;

fn app(allowed_hosts: &[&str]) -> Router {
    let mut config = InertiaConfig::new(|v: &RootView<'_>| Ok(v.body.to_string()));
    config.allowed_redirect_hosts = allowed_hosts.iter().map(ToString::to_string).collect();
    Router::new()
        .route(
            "/back",
            post(|i: Inertia| async move { i.back().into_response() }),
        )
        .route(
            "/back-or",
            post(|i: Inertia| async move { i.back_or("/dashboard").into_response() }),
        )
        .layer(Extension(config))
}

async fn location(
    path: &str,
    host: Option<&str>,
    referer: Option<&str>,
    allowed: &[&str],
) -> String {
    let mut req = Request::post(path);
    if let Some(h) = host {
        req = req.header(header::HOST, h);
    }
    if let Some(r) = referer {
        req = req.header(header::REFERER, r);
    }
    let res = app(allowed)
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), 302);
    res.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn same_site_referers_become_relative_urls() {
    let cases = [
        ("app.test", "http://app.test/form?step=2", "/form?step=2"),
        ("app.test", "https://app.test/form", "/form"),
        ("app.test", "https://APP.test/x", "/x"),
        ("app.test:8080", "http://app.test:8080/a", "/a"),
        ("app.test", "http://app.test", "/"),
        ("app.test", "/local/path?q=1", "/local/path?q=1"),
    ];
    for (host, referer, expected) in cases {
        assert_eq!(
            location("/back", Some(host), Some(referer), &[]).await,
            expected,
            "{referer}"
        );
    }
}

#[tokio::test]
async fn other_sites_and_tricks_fall_back() {
    let cases = [
        "https://evil.example/phish",
        "http://app.test:9999/other-port",
        "http://app.test@evil.example/userinfo-trick",
        "//evil.example/protocol-relative",
        "/\\\\evil.example/backslash",
        "javascript:alert(1)",
        "ftp://app.test/file",
        "not a url",
    ];
    for referer in cases {
        assert_eq!(
            location("/back", Some("app.test"), Some(referer), &[]).await,
            "/",
            "{referer}"
        );
    }
}

#[tokio::test]
async fn missing_referer_or_host_falls_back() {
    assert_eq!(location("/back", Some("app.test"), None, &[]).await, "/");
    assert_eq!(
        location("/back", None, Some("http://app.test/a"), &[]).await,
        "/"
    );
}

#[tokio::test]
async fn allowed_hosts_cover_proxies_that_rewrite_host() {
    let allowed = ["app.example.com"];
    assert_eq!(
        location(
            "/back",
            Some("127.0.0.1:5150"),
            Some("https://app.example.com/settings"),
            &allowed
        )
        .await,
        "/settings"
    );
    assert_eq!(
        location(
            "/back",
            Some("127.0.0.1:5150"),
            Some("https://evil.example/"),
            &allowed
        )
        .await,
        "/"
    );
}

#[tokio::test]
async fn back_or_uses_its_fallback() {
    assert_eq!(
        location(
            "/back-or",
            Some("app.test"),
            Some("https://evil.example/"),
            &[]
        )
        .await,
        "/dashboard"
    );
    assert_eq!(
        location("/back-or", Some("app.test"), Some("http://app.test/p"), &[]).await,
        "/p"
    );
}
