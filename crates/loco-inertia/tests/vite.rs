use axum::{body::Body, http::Request, routing::get, Router};
use http_body_util::BodyExt;
use loco_inertia::{apply, Inertia, InertiaSettings, Vite, ViteSettings};
use serde_json::json;
use tower::ServiceExt;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn manifest() -> String {
    format!("{DIR}/manifest.json")
}

fn build() -> Vite {
    Vite::from_manifest(manifest(), "/static").unwrap()
}

#[test]
fn entry_tags_include_css_preloads_and_script() {
    let tags = build().tags(Some("src/main.tsx")).unwrap();
    assert_eq!(
        tags,
        concat!(
            r#"<link rel="stylesheet" href="/static/assets/main-1b2c.css">"#,
            r#"<link rel="stylesheet" href="/static/assets/vendor-5e6f.css">"#,
            r#"<link rel="modulepreload" href="/static/assets/vendor-9c1d.js">"#,
            r#"<link rel="modulepreload" href="/static/assets/shared-77aa.js">"#,
            r#"<script type="module" src="/static/assets/main-4f2a.js"></script>"#,
        )
    );
    // Dynamic imports stay lazy.
    assert!(!tags.contains("Lazy"));
}

#[test]
fn unknown_entry_and_missing_manifest_are_errors() {
    assert!(build().tags(Some("src/nope.tsx")).is_err());
    assert!(build().tags(None).is_err(), "no default entry configured");
    let err = Vite::from_manifest("missing/manifest.json", "/")
        .unwrap_err()
        .to_string();
    assert!(err.contains("vite build"), "{err}");
}

#[test]
fn version_is_a_stable_hash_of_the_manifest() {
    let a = Vite::from_manifest_bytes(br#"{"a.js":{"file":"a-1.js"}}"#, "/").unwrap();
    let b = Vite::from_manifest_bytes(br#"{"a.js":{"file":"a-1.js"}}"#, "/").unwrap();
    let c = Vite::from_manifest_bytes(br#"{"a.js":{"file":"a-2.js"}}"#, "/").unwrap();
    assert_eq!(a.version(), b.version());
    assert_ne!(a.version(), c.version());
    assert_eq!(a.version().unwrap().len(), 16);
}

#[test]
fn dev_server_tags() {
    let vite =
        Vite::dev("http://localhost:5173/", true).with_default_entry(Some("src/main.tsx".into()));
    let tags = vite.tags(None).unwrap();
    assert!(tags.starts_with(r#"<script type="module">import RefreshRuntime from "http://localhost:5173/@react-refresh";"#));
    assert!(tags.ends_with(concat!(
        r#"<script type="module" src="http://localhost:5173/@vite/client"></script>"#,
        r#"<script type="module" src="http://localhost:5173/src/main.tsx"></script>"#,
    )));
    assert_eq!(vite.version(), None);

    let plain = Vite::dev("http://localhost:5173", false)
        .tags(Some("src/main.tsx"))
        .unwrap();
    assert!(!plain.contains("RefreshRuntime"));
}

#[test]
fn dev_server_wins_over_manifest() {
    let vite = Vite::from_settings(&ViteSettings {
        dev_server: Some("http://localhost:5173".into()),
        manifest: Some("does/not/exist.json".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(vite.version(), None);
}

async fn render_html(settings: InertiaSettings) -> (String, Option<String>) {
    let config = settings.into_config().unwrap();
    let version = config.version.clone();
    let app = apply(
        Router::new().route(
            "/",
            get(|i: Inertia| async move { i.render("Home", ()).await }),
        ),
        config,
    );
    let res = app
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    (html, version)
}

#[tokio::test]
async fn root_template_calls_vite_and_version_comes_from_the_manifest() {
    let settings = InertiaSettings::from_loco_settings(Some(&json!({ "inertia": {
        "root_template": format!("{DIR}/vite_root.html"),
        "vite": { "entry": "src/main.tsx", "manifest": manifest(), "base": "/static/" },
    }})))
    .unwrap();
    let (html, version) = render_html(settings).await;

    assert_eq!(version.as_deref(), build().version());
    assert!(
        html.starts_with(r#"<head><link rel="stylesheet" href="/static/assets/main-1b2c.css">"#),
        "{html}"
    );
    assert!(html.contains(r#"|<link rel="stylesheet" href="/static/assets/vendor-5e6f.css"><link rel="modulepreload" href="/static/assets/vendor-9c1d.js"><link rel="modulepreload" href="/static/assets/shared-77aa.js"><script type="module" src="/static/assets/admin-aa11.js"></script></head>"#), "{html}");
    assert!(html.contains(&format!(r#""version":"{}""#, version.unwrap())));
}

#[tokio::test]
async fn explicit_version_wins_over_the_manifest() {
    let settings = InertiaSettings::from_loco_settings(Some(&json!({ "inertia": {
        "version": "v42",
        "root_template": format!("{DIR}/vite_root.html"),
        "vite": { "entry": "src/main.tsx", "manifest": manifest() },
    }})))
    .unwrap();
    let (_, version) = render_html(settings).await;
    assert_eq!(version.as_deref(), Some("v42"));
}
