use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::from_fn,
    response::Response,
    routing::get,
    Extension, Router,
};
use http_body_util::BodyExt;
use inertia_axum::{inertia_middleware, Inertia, InertiaConfig, Prop, Props, RootView};
use serde_json::{json, Value};
use tower::ServiceExt;

/// Records which lazy props were resolved.
#[derive(Clone, Default)]
struct Calls(Arc<Mutex<Vec<&'static str>>>);

impl Calls {
    fn lazy(
        &self,
        name: &'static str,
        value: Value,
    ) -> impl FnOnce() -> std::future::Ready<Result<Value, Infallible>> + Send + 'static {
        let calls = self.clone();
        move || {
            calls.0.lock().unwrap().push(name);
            std::future::ready(Ok(value))
        }
    }

    fn take(&self) -> Vec<&'static str> {
        let mut v = std::mem::take(&mut *self.0.lock().unwrap());
        v.sort_unstable();
        v
    }
}

fn feed_props(calls: &Calls) -> Props {
    Props::new()
        .with("title", "Feed")
        .with("user", json!({ "name": "Ann", "email": "ann@example.com" }))
        .with("auth", Prop::always(json!({ "id": 1 })))
        .with(
            "posts",
            Prop::lazy(calls.lazy("posts", json!([{ "id": 1 }])))
                .merge()
                .match_on("id"),
        )
        .with("notifications", Prop::value(["n1"]).prepend())
        .with(
            "conversations",
            Prop::value(json!({ "data": [] })).deep_merge(),
        )
        .with(
            "categories",
            Prop::optional(calls.lazy("categories", json!(["a", "b"]))),
        )
        .with("stats", Prop::defer(calls.lazy("stats", json!(42))))
        .with(
            "comments",
            Prop::defer(calls.lazy("comments", json!([])))
                .group("sidebar")
                .merge(),
        )
        .with(
            "sidebar",
            Props::new()
                .with(
                    "links",
                    Prop::optional(calls.lazy("sidebar.links", json!(["x"]))),
                )
                .with(
                    "count",
                    Prop::defer(calls.lazy("sidebar.count", json!(3))).group("sidebar"),
                )
                .with("label", "Side"),
        )
}

fn app(calls: Calls) -> Router {
    Router::new()
        .route(
            "/feed",
            get(move |i: Inertia| {
                let calls = calls.clone();
                async move { i.render("Feed", feed_props(&calls)).await }
            }),
        )
        .route(
            "/scalar",
            get(|i: Inertia| async move { i.render("Bad", 5).await }),
        )
        .route(
            "/failing",
            get(|i: Inertia| async move {
                i.render(
                    "Failing",
                    Props::new().with(
                        "boom",
                        Prop::lazy(|| async { Err::<(), _>("db password leaked") }),
                    ),
                )
                .await
            }),
        )
        .layer(from_fn(inertia_middleware))
        .layer(Extension(InertiaConfig::new(|v: &RootView<'_>| {
            Ok(v.body.to_string())
        })))
}

async fn json_page(res: Response) -> Value {
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn inertia_get(uri: &str, headers: &[(&str, &str)]) -> Request<Body> {
    let mut req = Request::get(uri).header("X-Inertia", "true");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.body(Body::empty()).unwrap()
}

fn partial(only: &str, except: &str) -> Vec<(&'static str, String)> {
    let mut h = vec![("X-Inertia-Partial-Component", "Feed".to_string())];
    if !only.is_empty() {
        h.push(("X-Inertia-Partial-Data", only.to_string()));
    }
    if !except.is_empty() {
        h.push(("X-Inertia-Partial-Except", except.to_string()));
    }
    h
}

async fn get_feed(calls: &Calls, headers: Vec<(&str, String)>) -> Value {
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    json_page(
        app(calls.clone())
            .oneshot(inertia_get("/feed", &headers))
            .await
            .unwrap(),
    )
    .await
}

fn keys(v: &Value) -> Vec<&str> {
    let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort_unstable();
    k
}

#[tokio::test]
async fn full_visit_skips_optional_and_announces_deferred() {
    let calls = Calls::default();
    let page = get_feed(&calls, vec![]).await;

    assert_eq!(
        keys(&page["props"]),
        [
            "auth",
            "conversations",
            "errors",
            "notifications",
            "posts",
            "sidebar",
            "title",
            "user"
        ]
    );
    assert_eq!(page["props"]["sidebar"], json!({ "label": "Side" }));
    assert_eq!(
        page["deferredProps"],
        json!({ "default": ["stats"], "sidebar": ["comments", "sidebar.count"] })
    );
    assert_eq!(page["mergeProps"], json!(["posts"]));
    assert_eq!(page["prependProps"], json!(["notifications"]));
    assert_eq!(page["deepMergeProps"], json!(["conversations"]));
    assert_eq!(page["matchPropsOn"], json!(["posts.id"]));
    // Optional and deferred resolvers never ran.
    assert_eq!(calls.take(), ["posts"]);
}

#[tokio::test]
async fn full_visit_html_embeds_the_same_page() {
    let calls = Calls::default();
    let res = app(calls.clone())
        .oneshot(Request::get("/feed").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let html =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(
        html.contains(
            r#""deferredProps":{"default":["stats"],"sidebar":["comments","sidebar.count"]}"#
        ),
        "{html}"
    );
    assert!(!html.contains("categories"));
}

#[tokio::test]
async fn partial_only_returns_requested_and_always_props() {
    let calls = Calls::default();
    let page = get_feed(&calls, partial("stats,categories", "")).await;

    assert_eq!(
        keys(&page["props"]),
        ["auth", "categories", "errors", "stats"]
    );
    assert_eq!(page["props"]["stats"], 42);
    assert_eq!(page["props"]["categories"], json!(["a", "b"]));
    assert!(page.get("deferredProps").is_none());
    assert!(page.get("mergeProps").is_none());
    assert_eq!(calls.take(), ["categories", "stats"]);
}

#[tokio::test]
async fn partial_loads_a_deferred_group() {
    let calls = Calls::default();
    let page = get_feed(&calls, partial("comments,sidebar.count", "")).await;

    assert_eq!(
        keys(&page["props"]),
        ["auth", "comments", "errors", "sidebar"]
    );
    assert_eq!(page["props"]["sidebar"], json!({ "count": 3 }));
    assert_eq!(page["mergeProps"], json!(["comments"]));
    assert_eq!(calls.take(), ["comments", "sidebar.count"]);
}

#[tokio::test]
async fn partial_except_removes_props() {
    let calls = Calls::default();
    let page = get_feed(&calls, partial("", "posts,sidebar,user.email,auth")).await;

    // With no `only`, everything is a candidate (optional and deferred included), minus
    // `except`; always props survive `except`.
    assert_eq!(
        keys(&page["props"]),
        [
            "auth",
            "categories",
            "comments",
            "conversations",
            "errors",
            "notifications",
            "stats",
            "title",
            "user"
        ]
    );
    assert_eq!(page["props"]["user"], json!({ "name": "Ann" }));
    assert!(!calls.take().contains(&"posts"));
}

#[tokio::test]
async fn only_narrows_before_except() {
    let calls = Calls::default();
    let page = get_feed(&calls, partial("user,sidebar", "sidebar.links")).await;

    assert_eq!(keys(&page["props"]), ["auth", "errors", "sidebar", "user"]);
    // Requesting a parent includes its optional and deferred children…
    assert_eq!(
        page["props"]["sidebar"],
        json!({ "count": 3, "label": "Side" })
    );
    assert_eq!(calls.take(), ["sidebar.count"]);
}

#[tokio::test]
async fn dot_paths_reach_into_nested_props_and_plain_values() {
    let calls = Calls::default();
    let page = get_feed(&calls, partial("sidebar.links,user.name", "")).await;

    assert_eq!(page["props"]["sidebar"], json!({ "links": ["x"] }));
    assert_eq!(page["props"]["user"], json!({ "name": "Ann" }));
    assert_eq!(calls.take(), ["sidebar.links"]);
}

#[tokio::test]
async fn reset_drops_merge_metadata() {
    let calls = Calls::default();
    let mut headers = partial("posts,notifications", "");
    headers.push(("X-Inertia-Reset", "posts".to_string()));
    let page = get_feed(&calls, headers).await;

    assert_eq!(page["props"]["posts"], json!([{ "id": 1 }]));
    assert!(page.get("mergeProps").is_none());
    assert!(page.get("matchPropsOn").is_none());
    assert_eq!(page["prependProps"], json!(["notifications"]));
}

#[tokio::test]
async fn partial_headers_for_another_component_mean_a_full_visit() {
    let calls = Calls::default();
    let page = get_feed(
        &calls,
        vec![
            ("X-Inertia-Partial-Component", "Other".to_string()),
            ("X-Inertia-Partial-Data", "stats".to_string()),
        ],
    )
    .await;
    assert!(page["props"].get("title").is_some());
    assert!(page["props"].get("stats").is_none());
    assert!(page.get("deferredProps").is_some());
}

#[tokio::test]
async fn partial_headers_are_ignored_without_x_inertia() {
    let calls = Calls::default();
    let req = Request::get("/feed")
        .header("X-Inertia-Partial-Component", "Feed")
        .header("X-Inertia-Partial-Data", "stats")
        .body(Body::empty())
        .unwrap();
    let res = app(calls.clone()).oneshot(req).await.unwrap();
    let html =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(html.contains(r#""title":"Feed""#));
    assert!(!calls.take().contains(&"stats"));
}

#[tokio::test]
async fn serialize_props_must_be_an_object() {
    let res = app(Calls::default())
        .oneshot(inertia_get("/scalar", &[]))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn failing_lazy_prop_is_a_generic_500() {
    let res = app(Calls::default())
        .oneshot(inertia_get("/failing", &[]))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains("password"));
}

#[tokio::test]
async fn lazy_default_props_are_skipped_when_not_requested() {
    let resolved = Arc::new(AtomicUsize::new(0));
    let counter = resolved.clone();
    let props = Props::new().with("a", 1).with(
        "expensive",
        Prop::lazy(move || async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Infallible>(0)
        }),
    );
    let req = inertia_axum::PropRequest {
        partial: true,
        only: vec!["a".into()],
        ..Default::default()
    };
    let out = props.resolve(&req).await.unwrap();
    assert_eq!(Value::Object(out.props), json!({ "a": 1 }));
    assert_eq!(resolved.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn later_props_override_earlier_ones_on_merge() {
    let shared = Props::new().with("app", "demo").with("user", "guest");
    let page = Props::new().with("user", "ann");
    let out = shared
        .merge(page)
        .resolve(&Default::default())
        .await
        .unwrap();
    assert_eq!(
        Value::Object(out.props),
        json!({ "app": "demo", "user": "ann" })
    );
}

#[derive(serde::Serialize)]
struct TypedProps {
    title: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    stats: Option<u32>,
}

impl inertia_axum::InertiaPage for TypedProps {
    const COMPONENT: &'static str = "Typed/Show";
}

#[tokio::test]
async fn typed_pages_carry_their_component_and_accept_lazy_props() {
    let app = Router::new()
        .route(
            "/typed",
            get(|i: Inertia| async move {
                i.page(TypedProps {
                    title: "t",
                    stats: None,
                })
                .await
            }),
        )
        .route(
            "/typed-lazy",
            get(|i: Inertia| async move {
                i.page_with(
                    TypedProps {
                        title: "t",
                        stats: None,
                    },
                    |p| p.with("stats", Prop::defer(|| async { Ok::<_, Infallible>(7) })),
                )
                .await
            }),
        )
        .layer(Extension(InertiaConfig::new(|v: &RootView<'_>| {
            Ok(v.body.to_string())
        })));

    let page = json_page(
        app.clone()
            .oneshot(inertia_get("/typed", &[]))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(page["component"], "Typed/Show");
    assert_eq!(page["props"], json!({ "title": "t", "errors": {} }));

    let page = json_page(
        app.clone()
            .oneshot(inertia_get("/typed-lazy", &[]))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(page["deferredProps"], json!({ "default": ["stats"] }));

    let page = json_page(
        app.oneshot(inertia_get(
            "/typed-lazy",
            &[
                ("X-Inertia-Partial-Component", "Typed/Show"),
                ("X-Inertia-Partial-Data", "stats"),
            ],
        ))
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(page["props"], json!({ "stats": 7, "errors": {} }));
}

#[tokio::test]
async fn shared_props_can_be_extended_from_a_struct() {
    #[derive(serde::Serialize)]
    struct AppShared {
        app_name: &'static str,
        version: u32,
    }
    let shared = inertia_axum::SharedProps::default();
    shared.insert("user", "ann");
    shared
        .extend(AppShared {
            app_name: "demo",
            version: 2,
        })
        .unwrap();
    assert!(shared.extend(5).is_err(), "must be an object");
    let dbg = format!("{shared:?}");
    assert!(
        dbg.contains("app_name") && dbg.contains("user") && dbg.contains("version"),
        "{dbg}"
    );
}

#[tokio::test]
async fn sibling_lazy_props_resolve_concurrently() {
    let slow = |v: u32| {
        Prop::lazy(move || async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            Ok::<_, std::convert::Infallible>(v)
        })
    };
    let props = Props::new()
        .with("a", slow(1))
        .with("b", slow(2))
        .with("nested", Props::new().with("c", slow(3)));
    let started = std::time::Instant::now();
    let resolved = props
        .resolve(&inertia_axum::PropRequest::default())
        .await
        .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_millis(350),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        Value::Object(resolved.props),
        json!({ "a": 1, "b": 2, "nested": { "c": 3 } })
    );
}
