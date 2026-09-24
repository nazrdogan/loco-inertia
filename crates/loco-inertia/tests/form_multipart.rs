//! `InertiaForm` with multipart (files) and urlencoded bodies.

use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use http_body_util::BodyExt;
use loco_inertia::{
    apply, CookieFlash, CookieKey, Inertia, InertiaConfig, InertiaForm, Setup, UploadedFile,
};
use serde::Deserialize;
use tower::ServiceExt;

#[derive(Debug, Deserialize)]
struct Address {
    street: String,
    city: String,
}

#[derive(Debug, Deserialize, validator::Validate)]
struct Profile {
    #[validate(length(min = 2, message = "Name is too short."))]
    name: String,
    age: u32,
    #[serde(default)]
    newsletter: bool,
    avatar: Option<UploadedFile>,
    #[serde(default)]
    photos: Vec<UploadedFile>,
    #[serde(default)]
    tags: Vec<String>,
    address: Option<Address>,
    bio: Option<String>,
}

type Seen = Arc<Mutex<Option<Profile>>>;

fn app(seen: Seen) -> Router {
    let router = Router::new().route(
        "/profile",
        post(
            move |i: Inertia, InertiaForm(profile): InertiaForm<Profile>| {
                let seen = seen.clone();
                async move {
                    *seen.lock().unwrap() = Some(profile);
                    i.redirect("/profile").into_response()
                }
            },
        ),
    );
    apply(
        router,
        Setup {
            config: InertiaConfig::new(|v: &loco_inertia::RootView<'_>| Ok(v.body.to_string())),
            flash: Some(CookieFlash::new(CookieKey::from(&[6u8; 64]))),
            csrf: None,
        },
    )
}

const BOUNDARY: &str = "XyZzY";

enum Part<'a> {
    Text(&'a str, &'a str),
    File(&'a str, &'a str, &'a str, &'a [u8]),
}

fn multipart(parts: &[Part<'_>]) -> Vec<u8> {
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match part {
            Part::Text(name, value) => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                        .as_bytes(),
                );
            }
            Part::File(name, file_name, content_type, bytes) => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n").as_bytes(),
                );
                body.extend_from_slice(bytes);
                body.extend_from_slice(b"\r\n");
            }
        }
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

async fn send(
    content_type: &str,
    body: Vec<u8>,
    headers: &[(&str, &str)],
) -> (Response, Option<Profile>, String) {
    let seen: Seen = Default::default();
    let mut req = Request::post("/profile")
        .header(header::CONTENT_TYPE, content_type)
        .header("X-Inertia", "true")
        .header(header::HOST, "app.test")
        .header(header::REFERER, "http://app.test/profile/edit");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut res = app(seen.clone())
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let bytes = std::mem::take(res.body_mut())
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let profile = seen.lock().unwrap().take();
    (res, profile, String::from_utf8_lossy(&bytes).to_string())
}

fn multipart_type() -> String {
    format!("multipart/form-data; boundary={BOUNDARY}")
}

#[tokio::test]
async fn multipart_fields_and_files_become_the_struct() {
    let body = multipart(&[
        Part::Text("name", "Ann"),
        Part::Text("age", "42"),
        Part::Text("newsletter", "1"),
        Part::File("avatar", "me.png", "image/png", b"\x89PNG..."),
        Part::File("photos[0]", "a.jpg", "image/jpeg", b"aaa"),
        Part::File("photos[1]", "b.jpg", "image/jpeg", b"bbbb"),
        Part::Text("tags[0]", "rust"),
        Part::Text("tags[1]", "loco"),
        Part::Text("address[street]", "Main St 1"),
        Part::Text("address[city]", "Istanbul"),
        Part::Text("bio", ""),
    ]);
    let (res, profile, _) = send(&multipart_type(), body, &[]).await;
    assert_eq!(res.status(), StatusCode::FOUND);
    let p = profile.expect("handler ran");
    assert_eq!((p.name.as_str(), p.age, p.newsletter), ("Ann", 42, true));
    let avatar = p.avatar.unwrap();
    assert_eq!(avatar.file_name.as_deref(), Some("me.png"));
    assert_eq!(avatar.content_type.as_deref(), Some("image/png"));
    assert_eq!(&avatar.bytes[..], b"\x89PNG...");
    assert_eq!(
        p.photos.iter().map(|f| f.bytes.len()).collect::<Vec<_>>(),
        [3, 4]
    );
    assert_eq!(p.tags, ["rust", "loco"]);
    let address = p.address.unwrap();
    assert_eq!(
        (address.street.as_str(), address.city.as_str()),
        ("Main St 1", "Istanbul")
    );
    assert_eq!(p.bio, None, "an empty field is None");
}

#[tokio::test]
async fn an_empty_file_input_is_none() {
    let body = multipart(&[
        Part::Text("name", "Ann"),
        Part::Text("age", "1"),
        Part::File("avatar", "", "application/octet-stream", b""),
    ]);
    let (_, profile, _) = send(&multipart_type(), body, &[]).await;
    assert!(profile.unwrap().avatar.is_none());
}

#[tokio::test]
async fn text_cannot_pose_as_a_file() {
    let body = multipart(&[
        Part::Text("name", "Ann"),
        Part::Text("age", "1"),
        Part::File("photos[0]", "a.jpg", "image/jpeg", b"secret"),
        // Tries to point `avatar` at the uploaded photo.
        Part::Text("avatar[$inertia_file]", "0"),
        // The marker text as a plain value stays text.
        Part::Text("bio", "$inertia_file0"),
    ]);
    let (res, profile, _) = send(&multipart_type(), body, &[]).await;
    assert_eq!(res.status(), StatusCode::FOUND);
    let p = profile.unwrap();
    assert!(p.avatar.is_none());
    assert_eq!(p.bio.as_deref(), Some("$inertia_file0"));
    assert_eq!(p.photos.len(), 1);
}

#[tokio::test]
async fn urlencoded_forms_work_too() {
    let body = b"name=Ann&age=7&newsletter=on&tags%5B%5D=a&tags%5B%5D=b".to_vec();
    let (res, profile, _) = send("application/x-www-form-urlencoded", body, &[]).await;
    assert_eq!(res.status(), StatusCode::FOUND);
    let p = profile.unwrap();
    assert_eq!((p.age, p.newsletter), (7, true));
    assert_eq!(p.tags, ["a", "b"]);

    let (_, profile, _) = send(
        "application/x-www-form-urlencoded",
        b"name=Ann&age=7&newsletter=0".to_vec(),
        &[],
    )
    .await;
    assert!(!profile.unwrap().newsletter);
}

/// Nesting is bounded: building and deserializing the field tree recurses per level, and
/// an unbounded name used to overflow the stack and abort the whole process.
#[tokio::test]
async fn deeply_nested_field_names_are_rejected() {
    let body = format!("name=Ann&age=7&bio{}=x", "[a]".repeat(100_000)).into_bytes();
    let (res, profile, body) = send("application/x-www-form-urlencoded", body, &[]).await;
    assert!(profile.is_none());
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("nest at most"), "{body}");

    let body = multipart(&[
        Part::Text("name", "Ann"),
        Part::Text("age", "7"),
        Part::Text(&format!("bio{}", "[a]".repeat(40)), "x"),
    ]);
    let (res, profile, _) = send(&multipart_type(), body, &[]).await;
    assert!(profile.is_none());
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn appended_items_follow_explicit_indices() {
    let body = b"name=Ann&age=7&tags%5B1%5D=a&tags%5B%5D=b".to_vec();
    let (res, profile, _) = send("application/x-www-form-urlencoded", body, &[]).await;
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(profile.unwrap().tags, ["a", "b"]);
}

#[tokio::test]
async fn invalid_multipart_input_redirects_back_with_errors() {
    let body = multipart(&[Part::Text("name", "A"), Part::Text("age", "3")]);
    let (res, profile, _) = send(&multipart_type(), body, &[]).await;
    assert!(profile.is_none(), "handler must not run");
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(res.headers()[header::LOCATION], "/profile/edit");
    assert!(res.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .starts_with("inertia_flash="));
}

#[tokio::test]
async fn precognition_works_with_multipart() {
    let body = multipart(&[Part::Text("name", "A"), Part::Text("age", "3")]);
    let (res, profile, body) = send(
        &multipart_type(),
        body,
        &[
            ("Precognition", "true"),
            ("Precognition-Validate-Only", "name"),
        ],
    )
    .await;
    assert!(profile.is_none());
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("Name is too short."), "{body}");
}

#[tokio::test]
async fn values_of_the_wrong_type_are_rejected() {
    let body = multipart(&[Part::Text("name", "Ann"), Part::Text("age", "abc")]);
    let (res, profile, body) = send(&multipart_type(), body, &[]).await;
    assert!(profile.is_none());
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("invalid number"), "{body}");
}

/// What `@inertiajs/core`'s `objectToFormData` actually sends by default: arrays as
/// repeated `name[]` fields, booleans as `1`/`0`, `null` as an empty string.
#[tokio::test]
async fn inertia_default_bracket_format() {
    let body = multipart(&[
        Part::Text("name", "Ann"),
        Part::Text("age", "30"),
        Part::Text("newsletter", "0"),
        Part::File("photos[]", "a.jpg", "image/jpeg", b"a"),
        Part::File("photos[]", "b.jpg", "image/jpeg", b"bb"),
        Part::Text("tags[]", "x"),
        Part::Text("tags[]", "y"),
        Part::Text("bio", ""),
    ]);
    let (res, profile, _) = send(&multipart_type(), body, &[]).await;
    assert_eq!(res.status(), StatusCode::FOUND);
    let p = profile.unwrap();
    assert!(!p.newsletter);
    assert_eq!(
        p.photos.iter().map(|f| f.bytes.len()).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(p.tags, ["x", "y"]);
    assert_eq!(p.bio, None);
}

#[test]
fn uploaded_files_serialize_metadata_only() {
    let file = UploadedFile {
        file_name: Some("me.png".into()),
        content_type: Some("image/png".into()),
        bytes: axum::body::Bytes::from_static(b"secret-bytes"),
    };
    let json = serde_json::to_value(&file).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "file_name": "me.png", "content_type": "image/png", "size": 12 })
    );
}
