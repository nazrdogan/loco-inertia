//! Once props: resolved once, reused by the client, skipped via X-Inertia-Except-Once-Props.

use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{body::Body, http::Request, routing::get, Extension, Router};
use http_body_util::BodyExt;
use inertia_axum::{Inertia, InertiaConfig, Prop, Props, RootView};
use serde_json::{json, Value};
use tower::ServiceExt;

type Calls = Arc<Mutex<Vec<&'static str>>>;

fn lazy(calls: &Calls, name: &'static str) -> Prop {
    let calls = calls.clone();
    Prop::lazy(move || async move {
        calls.lock().unwrap().push(name);
        Ok::<_, Infallible>(format!("{name}-value"))
    })
}

fn props(calls: &Calls, fresh: bool) -> Props {
    Props::new()
        .with("title", "Billing")
        .with("plans", lazy(calls, "plans").once())
        .with(
            "countries",
            lazy(calls, "countries")
                .once_as("geo")
                .expires_in(Duration::from_secs(60)),
        )
        .with("rates", lazy(calls, "rates").once().fresh(fresh))
        .with(
            "report",
            Prop::defer({
                let calls = calls.clone();
                move || async move {
                    calls.lock().unwrap().push("report");
                    Ok::<_, Infallible>(1)
                }
            })
            .once(),
        )
}

async fn get_page(
    headers: &[(&str, &str)],
    inertia: bool,
    fresh: bool,
) -> (Value, Vec<&'static str>) {
    let calls: Calls = Default::default();
    let c = calls.clone();
    let app = Router::new()
        .route(
            "/",
            get(move |i: Inertia| async move { i.render("Billing", props(&c, fresh)).await }),
        )
        .layer(Extension(InertiaConfig::new(|v: &RootView<'_>| {
            Ok(v.body.to_string())
        })));
    let mut req = Request::get("/");
    if inertia {
        req = req.header("X-Inertia", "true");
    }
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let body =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    let json = if inertia {
        body
    } else {
        let start = body.find('>').unwrap() + 1;
        body[start..body.find("</script>").unwrap()].to_string()
    };
    let mut calls = calls.lock().unwrap().clone();
    calls.sort_unstable();
    (serde_json::from_str(&json).unwrap(), calls)
}

const EXCEPT: &str = "X-Inertia-Except-Once-Props";

#[tokio::test]
async fn first_visit_resolves_and_describes_once_props() {
    let (page, calls) = get_page(&[], true, false).await;

    assert_eq!(calls, ["countries", "plans", "rates"]);
    assert_eq!(page["props"]["plans"], "plans-value");
    assert_eq!(
        page["onceProps"]["plans"],
        json!({ "prop": "plans", "expiresAt": null })
    );
    assert_eq!(page["onceProps"]["geo"]["prop"], "countries");
    // Deferred once props are announced as deferred, described when they load.
    assert!(page["onceProps"].get("report").is_none());
    assert_eq!(page["deferredProps"], json!({ "default": ["report"] }));
}

#[tokio::test]
async fn expiry_is_a_unix_timestamp_in_milliseconds() {
    let (page, _) = get_page(&[], true, false).await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let expires = page["onceProps"]["geo"]["expiresAt"].as_u64().unwrap();
    assert!(
        expires > now + 55_000 && expires <= now + 60_000,
        "{expires} vs {now}"
    );
}

#[tokio::test]
async fn once_props_the_client_has_are_not_resolved_or_sent() {
    let (page, calls) = get_page(&[(EXCEPT, "plans, geo")], true, false).await;

    assert_eq!(calls, ["rates"]);
    assert!(page["props"].get("plans").is_none() && page["props"].get("countries").is_none());
    assert_eq!(page["props"]["title"], "Billing");
    // Still described, so the client keeps (and copies over) its values.
    assert_eq!(page["onceProps"]["plans"]["prop"], "plans");
    assert_eq!(page["onceProps"]["geo"]["prop"], "countries");
}

#[tokio::test]
async fn deferred_once_props_the_client_has_are_not_deferred_again() {
    let (page, calls) = get_page(&[(EXCEPT, "report")], true, false).await;
    assert!(!calls.contains(&"report"));
    assert!(page.get("deferredProps").is_none(), "{page}");
    // Described, so the client keeps its value.
    assert_eq!(page["onceProps"]["report"]["prop"], "report");
}

#[tokio::test]
async fn fresh_once_props_are_always_resolved() {
    let (page, calls) = get_page(&[(EXCEPT, "rates")], true, true).await;
    assert_eq!(page["props"]["rates"], "rates-value");
    assert!(calls.contains(&"rates"));
}

#[tokio::test]
async fn an_explicit_partial_reload_refreshes_a_once_prop() {
    let (page, calls) = get_page(
        &[
            (EXCEPT, "plans"),
            ("X-Inertia-Partial-Component", "Billing"),
            ("X-Inertia-Partial-Data", "plans,report"),
        ],
        true,
        false,
    )
    .await;
    assert_eq!(calls, ["plans", "report"]);
    assert_eq!(page["props"]["plans"], "plans-value");
    assert_eq!(
        page["onceProps"]["report"],
        json!({ "prop": "report", "expiresAt": null })
    );
}

#[tokio::test]
async fn the_header_only_counts_on_inertia_requests() {
    let (page, calls) = get_page(&[(EXCEPT, "plans")], false, false).await;
    assert!(calls.contains(&"plans"));
    assert_eq!(page["props"]["plans"], "plans-value");
}
