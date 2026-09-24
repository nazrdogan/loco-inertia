//! Infinite scroll props: merge metadata, `scrollProps`, merge intent and reset.

use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use axum::{body::Body, http::Request, routing::get, Extension, Router};
use http_body_util::BodyExt;
use inertia_axum::{Inertia, InertiaConfig, Prop, Props, RootView, ScrollData, ScrollMeta};
use serde_json::{json, Value};
use tower::ServiceExt;

#[derive(serde::Deserialize)]
struct Q {
    page: Option<u64>,
}

fn items(page: u64) -> Vec<Value> {
    (1..=3)
        .map(|i| json!({ "id": (page - 1) * 3 + i }))
        .collect()
}

fn app(calls: Arc<AtomicUsize>) -> Router {
    Router::new()
        .route(
            "/posts",
            get(
                move |i: Inertia, axum::extract::Query(q): axum::extract::Query<Q>| {
                    let calls = calls.clone();
                    async move {
                        let page = q.page.unwrap_or(1);
                        let props = Props::new()
                            .with("title", "Posts")
                            .with(
                                "posts",
                                Prop::lazy(move || async move {
                                    calls.fetch_add(1, Ordering::SeqCst);
                                    Ok::<_, Infallible>(ScrollData::new(items(page)))
                                })
                                .scroll(ScrollMeta::numbered(page, 3))
                                .match_on("id"),
                            )
                            .with(
                                "users",
                                Prop::value(json!({ "items": ["u1"] })).scroll(
                                    ScrollMeta::cursor(Some("c2"), Some("c1"), None::<&str>)
                                        .page_name("users")
                                        .wrapper("items"),
                                ),
                            );
                        i.render("Posts/Index", props).await
                    }
                },
            ),
        )
        .layer(Extension(InertiaConfig::new(|v: &RootView<'_>| {
            Ok(v.body.to_string())
        })))
}

async fn get_page(uri: &str, headers: &[(&str, &str)]) -> (Value, usize) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut req = Request::get(uri).header("X-Inertia", "true");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = app(calls.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        serde_json::from_slice(&bytes).unwrap(),
        calls.load(Ordering::SeqCst),
    )
}

const PARTIAL: (&str, &str) = ("X-Inertia-Partial-Component", "Posts/Index");

#[tokio::test]
async fn full_visit_sends_items_merge_paths_and_scroll_metadata() {
    let (page, _) = get_page("/posts", &[]).await;

    assert_eq!(
        page["props"]["posts"],
        json!({ "data": [{ "id": 1 }, { "id": 2 }, { "id": 3 }] })
    );
    assert_eq!(page["mergeProps"], json!(["posts.data", "users.items"]));
    assert_eq!(page["matchPropsOn"], json!(["posts.data.id"]));
    assert_eq!(
        page["scrollProps"],
        json!({
            "posts": { "pageName": "page", "previousPage": null, "nextPage": 2, "currentPage": 1, "reset": false },
            "users": { "pageName": "users", "previousPage": "c1", "nextPage": null, "currentPage": "c2", "reset": false },
        })
    );
}

#[tokio::test]
async fn loading_the_next_page_appends() {
    let (page, calls) = get_page(
        "/posts?page=2",
        &[
            PARTIAL,
            ("X-Inertia-Partial-Data", "posts"),
            ("X-Inertia-Infinite-Scroll-Merge-Intent", "append"),
        ],
    )
    .await;

    assert_eq!(calls, 1);
    assert_eq!(page["props"]["posts"]["data"][0]["id"], 4);
    assert!(page["props"].get("title").is_none() && page["props"].get("users").is_none());
    assert_eq!(page["mergeProps"], json!(["posts.data"]));
    assert!(page.get("prependProps").is_none());
    assert_eq!(
        page["scrollProps"],
        json!({ "posts": { "pageName": "page", "previousPage": 1, "nextPage": 3, "currentPage": 2, "reset": false } })
    );
}

#[tokio::test]
async fn loading_the_previous_page_prepends() {
    let (page, _) = get_page(
        "/posts?page=1",
        &[
            PARTIAL,
            ("X-Inertia-Partial-Data", "posts"),
            ("X-Inertia-Infinite-Scroll-Merge-Intent", "prepend"),
        ],
    )
    .await;

    assert_eq!(page["prependProps"], json!(["posts.data"]));
    assert!(page.get("mergeProps").is_none());
    assert_eq!(page["matchPropsOn"], json!(["posts.data.id"]));
}

#[tokio::test]
async fn reset_replaces_the_items_and_flags_the_client() {
    let (page, _) = get_page(
        "/posts?page=1",
        &[
            PARTIAL,
            ("X-Inertia-Partial-Data", "posts"),
            ("X-Inertia-Reset", "posts"),
        ],
    )
    .await;

    assert!(page.get("mergeProps").is_none() && page.get("matchPropsOn").is_none());
    assert_eq!(page["scrollProps"]["posts"]["reset"], true);
}

#[tokio::test]
async fn scroll_props_left_out_of_a_partial_reload_are_not_resolved_or_described() {
    let (page, calls) = get_page("/posts", &[PARTIAL, ("X-Inertia-Partial-Data", "title")]).await;

    assert_eq!(calls, 0);
    assert!(page.get("scrollProps").is_none());
    assert!(page.get("mergeProps").is_none());
}

#[tokio::test]
async fn merge_intent_is_ignored_outside_partial_reloads() {
    let (page, _) = get_page(
        "/posts",
        &[("X-Inertia-Infinite-Scroll-Merge-Intent", "prepend")],
    )
    .await;
    assert_eq!(page["mergeProps"], json!(["posts.data", "users.items"]));
}

#[test]
fn numbered_pagination_edges() {
    let meta = |c, l| {
        let m = ScrollMeta::numbered(c, l);
        (m.clone(), m)
    };
    assert_eq!(
        meta(1, 1).0,
        ScrollMeta::cursor(Some(1u64), None::<u64>, None::<u64>)
    );
    assert_eq!(
        meta(3, 3).0,
        ScrollMeta::cursor(Some(3u64), Some(2u64), None::<u64>)
    );
    assert_eq!(
        meta(0, 5).0,
        ScrollMeta::cursor(Some(1u64), None::<u64>, Some(2u64)),
        "page 0 clamps to 1"
    );
}

#[derive(serde::Serialize)]
struct TypedFeed {
    title: String,
    posts: ScrollData<u32>,
}

impl inertia_axum::InertiaPage for TypedFeed {
    const COMPONENT: &'static str = "Feed";
}

#[tokio::test]
async fn typed_props_mark_a_field_as_scroll() {
    let app = Router::new()
        .route(
            "/",
            get(|i: Inertia| async move {
                let props = TypedFeed {
                    title: "t".into(),
                    posts: ScrollData::new(vec![1, 2]),
                };
                i.page_with(props, |p| {
                    p.scroll("posts", ScrollMeta::numbered(1, 2).match_on("id"))
                        // Unknown keys are logged and ignored.
                        .scroll("nope", ScrollMeta::numbered(1, 1))
                })
                .await
            }),
        )
        .layer(Extension(InertiaConfig::new(|v: &RootView<'_>| {
            Ok(v.body.to_string())
        })));
    let res = app
        .oneshot(
            Request::get("/")
                .header("X-Inertia", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let page: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(page["props"]["posts"], json!({ "data": [1, 2] }));
    assert_eq!(page["mergeProps"], json!(["posts.data"]));
    assert_eq!(page["matchPropsOn"], json!(["posts.data.id"]));
    assert_eq!(page["scrollProps"]["posts"]["nextPage"], 2);
    assert!(page["scrollProps"].get("nope").is_none());
}

#[cfg(feature = "ts")]
#[test]
fn scroll_data_has_a_typescript_type() {
    use ts_rs::TS;
    assert_eq!(
        ScrollData::<u32>::decl(&ts_rs::Config::default()),
        // Rust doc comments carry over as JSDoc.
        "type ScrollData<T> = { \n/**\n * The items of this page.\n */\ndata: Array<T>, };"
    );
}
