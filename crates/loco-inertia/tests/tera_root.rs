use axum::{body::Body, http::Request, routing::get, Router};
use http_body_util::BodyExt;
use loco_inertia::{apply, Inertia, InertiaSettings};
use serde_json::json;
use tower::ServiceExt;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/root.html");

#[test]
fn settings_default_when_missing() {
    let s = InertiaSettings::from_loco_settings(None).unwrap();
    assert!(s.version.is_none() && s.root_template.is_none() && s.app_id.is_none());
    let s = InertiaSettings::from_loco_settings(Some(&json!({ "other": 1 }))).unwrap();
    assert!(s.version.is_none());
}

#[test]
fn settings_reject_unknown_keys() {
    let err = InertiaSettings::from_loco_settings(Some(&json!({ "inertia": { "verion": "1" } })));
    assert!(err.is_err());
}

#[test]
fn missing_root_template_is_an_error() {
    let s = InertiaSettings {
        root_template: Some("does/not/exist.html".into()),
        ..Default::default()
    };
    assert!(s.into_config().is_err());
}

#[tokio::test]
async fn renders_tera_root_template_from_settings() {
    let settings = InertiaSettings::from_loco_settings(Some(&json!({
        "inertia": { "version": "abc", "root_template": FIXTURE, "app_id": "root" }
    })))
    .unwrap();
    let config = settings.into_config().unwrap();
    assert_eq!(config.version.as_deref(), Some("abc"));

    let app = apply(
        Router::new().route(
            "/",
            get(|i: Inertia| async move { i.render("Home", json!({ "title": "<b>" })).await }),
        ),
        config,
    );
    let res = app
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.headers()["vary"], "X-Inertia");
    let html =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();

    // `page` is autoescaped by Tera, the mount markup is inserted verbatim.
    assert!(html.contains("<title>Home — &lt;b&gt;</title>"), "{html}");
    assert!(
        html.contains(
            r#"<body><script data-page="root" type="application/json">{"component":"Home","#
        ),
        "{html}"
    );
    assert!(
        html.contains(r#""version":"abc"}</script><div id="root"></div></body>"#),
        "{html}"
    );
}

#[test]
fn flash_secret_must_be_long_enough() {
    let short = InertiaSettings {
        flash_secret: Some("too short".into()),
        ..Default::default()
    };
    assert!(short.cookie_flash().is_err());

    let ok = InertiaSettings::from_loco_settings(Some(&json!({
        "inertia": { "flash_secret": "x".repeat(64), "secure_cookies": true }
    })))
    .unwrap();
    assert!(ok.secure_cookies);
    assert!(ok.cookie_flash().is_ok());
    // Without a secret a random key is used…
    assert!(InertiaSettings::default().cookie_flash().is_ok());
    // …except where one is required (production).
    let required = InertiaSettings {
        require_flash_secret: true,
        ..Default::default()
    };
    let err = required.clone().setup().unwrap_err().to_string();
    assert!(err.contains("required in production"), "{err}");
    assert!(InertiaSettings {
        flash_secret: Some("x".repeat(64)),
        ..required
    }
    .cookie_flash()
    .is_ok());
}

#[test]
fn flash_secret_is_not_exposed_in_middleware_config() {
    let s = InertiaSettings {
        flash_secret: Some("x".repeat(64)),
        ..Default::default()
    };
    assert!(serde_json::to_value(&s)
        .unwrap()
        .get("flash_secret")
        .is_none());
}

#[derive(validator::Validate)]
struct Signup {
    #[validate(length(min = 3, message = "Name is too short."))]
    name: String,
    #[validate(email)]
    email: String,
    #[validate(range(min = 18))]
    age: u32,
}

#[test]
fn validator_errors_become_first_message_per_field() {
    use validator::Validate;

    let form = Signup {
        name: "Al".into(),
        email: "nope".into(),
        age: 30,
    };
    let errors = loco_inertia::validation_errors(&form.validate().unwrap_err());
    assert_eq!(
        serde_json::to_value(errors).unwrap(),
        json!({
            "name": "Name is too short.",
            "email": "The email field is invalid (email).",
        })
    );
}

#[test]
fn ssr_settings() {
    let parse = |ssr: serde_json::Value| {
        InertiaSettings::from_loco_settings(Some(&json!({
            "inertia": { "root_template": FIXTURE, "ssr": ssr }
        })))
        .unwrap()
        .into_config()
        .unwrap()
    };
    assert!(
        parse(json!({})).ssr.is_some(),
        "on when the section is present"
    );
    assert!(
        parse(json!({ "url": "http://127.0.0.1:9999", "timeout_ms": 500 }))
            .ssr
            .is_some()
    );
    assert!(parse(json!({ "enabled": false })).ssr.is_none());
    assert!(InertiaSettings {
        root_template: Some(FIXTURE.into()),
        ..Default::default()
    }
    .into_config()
    .unwrap()
    .ssr
    .is_none());
}

#[tokio::test]
async fn ssr_head_is_available_to_the_tera_template() {
    struct Fake;
    impl loco_inertia::inertia_core::SsrRenderer for Fake {
        fn render<'a>(
            &'a self,
            _page: &'a loco_inertia::Page,
        ) -> loco_inertia::inertia_core::SsrFuture<'a> {
            Box::pin(async {
                Ok(loco_inertia::inertia_core::SsrResponse {
                    head: vec!["<title inertia>From SSR</title>".into()],
                    body: "<div id=\"app\" data-server-rendered=\"true\">hi</div>".into(),
                })
            })
        }
    }
    let path = format!(
        "{}/tests/fixtures/ssr_root.html",
        env!("CARGO_MANIFEST_DIR")
    );
    let config = loco_inertia::tera_root_template_file(path)
        .unwrap()
        .with_ssr(Fake);
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
    assert_eq!(
        html.trim(),
        r#"<head><title inertia>From SSR</title></head><body><div id="app" data-server-rendered="true">hi</div></body>"#
    );
}

#[test]
fn setup_enables_csrf_unless_disabled_and_passes_encrypt_history() {
    let setup = |extra: serde_json::Value| {
        let mut inertia = json!({ "root_template": FIXTURE });
        inertia
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        InertiaSettings::from_loco_settings(Some(&json!({ "inertia": inertia })))
            .unwrap()
            .setup()
            .unwrap()
    };
    let default = setup(json!({}));
    assert!(default.csrf.is_some() && default.flash.is_some());
    assert!(!default.config.encrypt_history);

    let custom = setup(json!({ "csrf": { "exempt": ["/api/"] }, "encrypt_history": true }));
    assert!(format!("{:?}", custom.csrf.unwrap()).contains("/api/"));
    assert!(custom.config.encrypt_history);

    assert!(setup(json!({ "csrf": { "enabled": false } }))
        .csrf
        .is_none());
}
