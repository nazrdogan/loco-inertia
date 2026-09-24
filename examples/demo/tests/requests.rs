//! End-to-end checks through the real Loco app and middleware stack.

use demo::app::App;
use loco_rs::testing::prelude::*;
use serde_json::{json, Value};

#[tokio::test]
async fn first_visit_is_html_with_page_data() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/about").await;
        res.assert_status_ok();
        let html = res.text();
        assert!(html
            .contains(r#"<script data-page="app" type="application/json">{"component":"About""#));
        // Without SSR the title is a placeholder the client replaces from <Head>.
        assert!(html.contains("<title data-inertia>Loco + Inertia</title>"));
    })
    .await;
}

#[tokio::test]
async fn inertia_visit_is_json() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get("/about")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .await;
        res.assert_status_ok();
        assert_eq!(res.header("x-inertia"), "true");
        assert_eq!(res.json::<Value>()["component"], "About");
    })
    .await;
}

#[tokio::test]
async fn stale_version_409_goes_through_loco_middleware() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get("/feed?page=2")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "old")
            .await;
        assert_eq!(res.status_code(), 409);
        assert_eq!(res.header("x-inertia-location"), "/feed?page=2");
        // Set by Loco's own layers, which now wrap the Inertia middleware.
        assert!(res.maybe_header("x-request-id").is_some());
        assert_eq!(res.header("x-powered-by"), "loco.rs");
    })
    .await;
}

#[tokio::test]
async fn feed_defers_stats_and_hides_categories() {
    request::<App, _, _>(|request, _ctx| async move {
        let page: Value = request
            .get("/feed")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .await
            .json();
        let props = page["props"].as_object().unwrap();
        assert!(props.contains_key("posts") && !props.contains_key("stats"));
        assert!(!props.contains_key("categories"));
        assert_eq!(page["deferredProps"], json!({ "default": ["stats"] }));
    })
    .await;
}

/// Headers of the partial reload `<InfiniteScroll data="posts">` makes.
fn scroll_request() -> Vec<(&'static str, String)> {
    vec![
        ("X-Inertia", "true".into()),
        ("X-Inertia-Version", "dev".into()),
        ("X-Inertia-Partial-Component", "Feed".into()),
        ("X-Inertia-Partial-Data", "posts".into()),
    ]
}

#[tokio::test]
async fn feed_posts_are_an_infinite_scroll_prop() {
    request::<App, _, _>(|request, _ctx| async move {
        let page: Value = request
            .get("/feed")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .await
            .json();
        assert_eq!(page["props"]["posts"]["data"].as_array().unwrap().len(), 10);
        assert_eq!(page["mergeProps"], json!(["posts.data"]));
        assert_eq!(page["matchPropsOn"], json!(["posts.data.id"]));
        assert_eq!(
            page["scrollProps"]["posts"],
            json!({ "pageName": "page", "previousPage": null, "nextPage": 2, "currentPage": 1, "reset": false })
        );

        // What <InfiniteScroll> sends when the end of the list comes into view.
        let mut req = request.get("/feed?page=2");
        for (k, v) in scroll_request() {
            req = req.add_header(k, v);
        }
        let page: Value = req
            .add_header("X-Inertia-Infinite-Scroll-Merge-Intent", "append")
            .await
            .json();
        assert_eq!(page["props"]["posts"]["data"][0]["id"], 11);
        assert!(page["props"].get("stats").is_none());
        assert_eq!(page["scrollProps"]["posts"]["previousPage"], 1);
        assert_eq!(page["scrollProps"]["posts"]["nextPage"], 3);

        // The last page has no next page.
        let mut req = request.get("/feed?page=10");
        for (k, v) in scroll_request() {
            req = req.add_header(k, v);
        }
        let page: Value = req.await.json();
        assert_eq!(page["props"]["posts"]["data"][9]["id"], 100);
        assert_eq!(page["scrollProps"]["posts"]["nextPage"], Value::Null);
    })
    .await;
}

#[tokio::test]
async fn feed_filter_resets_the_scroll_prop() {
    request::<App, _, _>(|request, _ctx| async move {
        let page: Value = request
            .get("/feed?even=1")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .add_header("X-Inertia-Partial-Component", "Feed")
            .add_header("X-Inertia-Partial-Data", "posts,even_only")
            .add_header("X-Inertia-Reset", "posts")
            .await
            .json();
        assert_eq!(page["props"]["even_only"], true);
        assert_eq!(page["props"]["posts"]["data"][0]["id"], 2);
        assert_eq!(page["scrollProps"]["posts"]["reset"], true);
        assert_eq!(page["scrollProps"]["posts"]["nextPage"], 2);
        assert!(
            page.get("mergeProps").is_none(),
            "a reset replaces instead of merging"
        );
    })
    .await;
}

#[tokio::test]
async fn feed_optional_and_deferred_props_on_request() {
    request::<App, _, _>(|request, _ctx| async move {
        let page: Value = request
            .get("/feed")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .add_header("X-Inertia-Partial-Component", "Feed")
            .add_header("X-Inertia-Partial-Data", "stats,categories")
            .await
            .json();
        assert_eq!(page["props"]["stats"]["total_posts"], 100);
        assert_eq!(
            page["props"]["categories"],
            json!(["rust", "loco", "inertia"])
        );
        assert!(page.get("scrollProps").is_none());
    })
    .await;
}

#[tokio::test]
async fn shared_props_reach_every_page() {
    request::<App, _, _>(|request, _ctx| async move {
        let page: Value = request
            .get("/about")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .await
            .json();
        assert_eq!(page["props"]["app_name"], "Loco + Inertia");
        assert_eq!(page["props"]["errors"], json!({}));
        assert_eq!(page["sharedProps"], json!(["app_name", "loaded_at"]));
    })
    .await;
}

#[tokio::test]
async fn once_shared_prop_is_skipped_when_the_client_has_it() {
    request::<App, _, _>(|request, _ctx| async move {
        let page: Value = request
            .get("/about")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .await
            .json();
        assert!(page["props"]["loaded_at"]
            .as_str()
            .unwrap()
            .ends_with("UTC"));
        assert_eq!(
            page["onceProps"]["loaded_at"],
            json!({ "prop": "loaded_at", "expiresAt": null })
        );

        // Next visit: the client says it has it.
        let page: Value = request
            .get("/feed")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .add_header("X-Inertia-Except-Once-Props", "loaded_at")
            .await
            .json();
        assert!(page["props"].get("loaded_at").is_none());
        assert_eq!(page["onceProps"]["loaded_at"]["prop"], "loaded_at");
    })
    .await;
}

/// `name=value` of each Set-Cookie header.
fn cookies(res: &axum_test::TestResponse) -> Vec<String> {
    res.headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .collect()
}

/// A CSRF token issued on a page visit: `(cookie pair, header value)`.
async fn csrf_token(request: &loco_rs::TestServer) -> (String, String) {
    let res = request.get("/contact").await;
    let pair = cookies(&res)
        .into_iter()
        .find(|c| c.starts_with("XSRF-TOKEN="))
        .expect("XSRF-TOKEN cookie");
    let token = pair.split_once('=').unwrap().1.to_string();
    (pair, token)
}

#[tokio::test]
async fn unsafe_requests_need_the_csrf_token() {
    request::<App, _, _>(|request, _ctx| async move {
        let form =
            json!({ "name": "Ann", "email": "ann@example.com", "message": "Hello there, Loco!" });
        let res = request
            .post("/contact")
            .add_header("X-Inertia", "true")
            .json(&form)
            .await;
        assert_eq!(res.status_code(), 419);

        let (cookie, token) = csrf_token(&request).await;
        let res = request
            .post("/contact")
            .add_header("X-Inertia", "true")
            .add_header("Cookie", cookie)
            .add_header("X-XSRF-TOKEN", token)
            .json(&form)
            .await;
        assert_eq!(res.status_code(), 302);
    })
    .await;
}

#[tokio::test]
async fn contact_form_live_validation() {
    request::<App, _, _>(|request, _ctx| async move {
        let (cookie, token) = csrf_token(&request).await;
        let res = request
            .post("/contact")
            .add_header("Precognition", "true")
            .add_header("Precognition-Validate-Only", "email")
            .add_header("Cookie", cookie.clone())
            .add_header("X-XSRF-TOKEN", token.clone())
            .json(&json!({ "name": "", "email": "nope", "message": "" }))
            .await;
        assert_eq!(res.status_code(), 422);
        assert_eq!(res.header("precognition"), "true");
        assert_eq!(
            res.json::<Value>()["errors"],
            json!({ "email": "That does not look like an email address." })
        );

        let res = request
            .post("/contact")
            .add_header("Precognition", "true")
            .add_header("Precognition-Validate-Only", "email")
            .add_header("Cookie", cookie)
            .add_header("X-XSRF-TOKEN", token)
            .json(&json!({ "name": "", "email": "ann@example.com", "message": "" }))
            .await;
        assert_eq!(res.status_code(), 204);
        assert_eq!(res.header("precognition-success"), "true");
        // Nothing was flashed: the action did not run.
        assert!(cookies(&res)
            .iter()
            .all(|c| !c.starts_with("inertia_flash=")));
    })
    .await;
}

#[tokio::test]
async fn contact_page_is_encrypted_in_history() {
    request::<App, _, _>(|request, _ctx| async move {
        let page: Value = request
            .get("/contact")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .await
            .json();
        assert_eq!(page["encryptHistory"], true);
    })
    .await;
}

/// Submit the contact form with a valid CSRF token; returns the response and the cookies
/// to send on the next request.
async fn submit_contact(
    request: &loco_rs::TestServer,
    form: Value,
    referer: Option<&str>,
) -> (axum_test::TestResponse, String) {
    let (cookie, token) = csrf_token(request).await;
    let mut req = request
        .post("/contact")
        .add_header("X-Inertia", "true")
        .add_header("X-Inertia-Version", "dev")
        .add_header("Cookie", cookie.clone())
        .add_header("X-XSRF-TOKEN", token);
    if let Some(referer) = referer {
        req = req.add_header("Referer", referer.to_string());
    }
    let res = req.json(&form).await;
    let mut jar = cookies(&res);
    jar.push(cookie);
    (res, jar.join("; "))
}

#[tokio::test]
async fn invalid_contact_form_redirects_back_with_errors() {
    request::<App, _, _>(|request, _ctx| async move {
        let (res, cookies) = submit_contact(
            &request,
            json!({ "name": "A", "email": "nope", "message": "short" }),
            Some("http://localhost:5150/contact"),
        )
        .await;
        assert_eq!(res.status_code(), 302);
        // Same site: kept, as a relative URL.
        assert_eq!(res.header("location"), "/contact");

        let page: Value = request
            .get("/contact")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .add_header("Cookie", cookies)
            .await
            .json();
        assert_eq!(
            page["props"]["errors"],
            json!({
                "name": "Please tell us your name.",
                "email": "That does not look like an email address.",
                "message": "The message needs at least 10 characters.",
            })
        );
    })
    .await;
}

#[tokio::test]
async fn valid_contact_form_flashes_a_message() {
    request::<App, _, _>(|request, _ctx| async move {
        let (res, cookies) = submit_contact(
            &request,
            json!({ "name": "Ann", "email": "ann@example.com", "message": "Hello there, Loco!" }),
            None,
        )
        .await;
        assert_eq!(res.status_code(), 302);
        assert_eq!(res.header("location"), "/contact");

        let page: Value = request
            .get("/contact")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .add_header("Cookie", cookies)
            .await
            .json();
        assert_eq!(
            page["flash"],
            json!({ "message": "Thanks Ann, we got your message!" })
        );
        assert_eq!(page["props"]["errors"], json!({}));
    })
    .await;
}

#[tokio::test]
async fn back_never_leaves_the_site() {
    request::<App, _, _>(|request, _ctx| async move {
        let (res, _) = submit_contact(
            &request,
            json!({ "name": "A", "email": "nope", "message": "short" }),
            Some("https://evil.example/phishing"),
        )
        .await;
        assert_eq!(res.status_code(), 302);
        assert_eq!(res.header("location"), "/");
    })
    .await;
}

/// A multipart body as `@inertiajs/core` builds it (`photos[]` per file).
fn upload_body(name: &str, files: &[(&str, &str, &str, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "----inertia-test";
    let mut body =
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n")
            .into_bytes();
    for (field, file_name, content_type, bytes) in files {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[tokio::test]
async fn file_upload_round_trip() {
    request::<App, _, _>(|request, _ctx| async move {
        let (cookie, token) = csrf_token(&request).await;
        let (content_type, body) = upload_body(
            "Nazir",
            &[
                ("avatar", "me.png", "image/png", b"\x89PNG-avatar"),
                ("photos[]", "a.png", "image/png", b"aaa"),
                ("photos[]", "b.png", "image/png", b"bbbb"),
            ],
        );
        let res = request
            .post("/upload")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .add_header("Cookie", cookie.clone())
            .add_header("X-XSRF-TOKEN", token)
            .content_type(&content_type)
            .bytes(body.into())
            .await;
        assert_eq!(res.status_code(), 302, "{}", res.text());
        let mut jar = cookies(&res);
        jar.push(cookie);

        let page: Value = request
            .get("/upload")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .add_header("Cookie", jar.join("; "))
            .await
            .json();
        assert_eq!(
            page["flash"]["message"],
            "Thanks Nazir: me.png (image/png, 11 bytes); 2 photo(s): a.png (image/png, 3 bytes), b.png (image/png, 4 bytes)"
        );
    })
    .await;
}

#[tokio::test]
async fn file_upload_validation_errors() {
    request::<App, _, _>(|request, _ctx| async move {
        let (cookie, token) = csrf_token(&request).await;
        let (content_type, body) = upload_body(
            "Nazir",
            &[("avatar", "notes.txt", "text/plain", b"not an image")],
        );
        let res = request
            .post("/upload")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .add_header("Cookie", cookie.clone())
            .add_header("X-XSRF-TOKEN", token)
            .content_type(&content_type)
            .bytes(body.into())
            .await;
        assert_eq!(res.status_code(), 302);
        let mut jar = cookies(&res);
        jar.push(cookie);
        let page: Value = request
            .get("/upload")
            .add_header("X-Inertia", "true")
            .add_header("X-Inertia-Version", "dev")
            .add_header("Cookie", jar.join("; "))
            .await
            .json();
        assert_eq!(
            page["props"]["errors"],
            json!({ "avatar": "Only images are allowed." })
        );
    })
    .await;
}
