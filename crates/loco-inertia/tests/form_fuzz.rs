//! Adversarial input for `InertiaForm`: whatever the body, the extractor answers with a
//! redirect (valid or invalid data) or a 4xx, and never panics or overflows the stack.

use std::collections::HashMap;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use loco_inertia::{apply, Inertia, InertiaConfig, InertiaForm, Setup, UploadedFile};
use proptest::prelude::*;
use serde::Deserialize;
use tower::ServiceExt;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
enum Role {
    Admin,
    User,
}

#[derive(Debug, Deserialize, validator::Validate)]
#[allow(dead_code)]
struct Item {
    name: String,
    qty: Option<u8>,
}

/// Every kind of field the lenient deserializer handles.
#[derive(Debug, Deserialize, validator::Validate)]
#[allow(dead_code)]
struct Everything {
    #[validate(length(min = 1))]
    name: String,
    age: Option<u32>,
    delta: Option<i8>,
    ratio: Option<f64>,
    #[serde(default)]
    agree: bool,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    items: Vec<Item>,
    #[serde(default)]
    meta: HashMap<String, String>,
    role: Option<Role>,
    avatar: Option<UploadedFile>,
    #[serde(default)]
    photos: Vec<UploadedFile>,
}

fn app() -> Router {
    let router = Router::new().route(
        "/f",
        post(
            |i: Inertia, InertiaForm(_): InertiaForm<Everything>| async move {
                i.redirect("/done").into_response()
            },
        ),
    );
    apply(
        router,
        Setup {
            config: InertiaConfig::new(|v: &loco_inertia::RootView<'_>| Ok(v.body.to_string())),
            flash: None,
            csrf: None,
        },
    )
}

fn send(content_type: &str, body: Vec<u8>, precognition: bool) -> StatusCode {
    let mut req = Request::post("/f")
        .header(header::CONTENT_TYPE, content_type)
        .header("X-Inertia", "true")
        .header(header::REFERER, "/form");
    if precognition {
        req = req.header("Precognition", "true");
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(app().oneshot(req.body(Body::from(body)).unwrap()))
        .unwrap()
        .status()
}

fn acceptable(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::FOUND
            | StatusCode::NO_CONTENT
            | StatusCode::BAD_REQUEST
            | StatusCode::PAYLOAD_TOO_LARGE
            | StatusCode::UNPROCESSABLE_ENTITY
            | StatusCode::UNSUPPORTED_MEDIA_TYPE
    )
}

/// Field names built from the parts that matter to the parser: known fields, indices,
/// `[]`, huge numbers, the file marker, stray brackets.
fn field_name() -> impl Strategy<Value = String> {
    let head = prop::sample::select(vec![
        "name",
        "age",
        "delta",
        "ratio",
        "agree",
        "tags",
        "items",
        "meta",
        "role",
        "avatar",
        "photos",
        "",
        "$inertia_file",
        "x",
    ]);
    let segment = prop::sample::select(vec![
        "[]",
        "[0]",
        "[1]",
        "[7]",
        "[18446744073709551616]",
        "[name]",
        "[qty]",
        "[$inertia_file]",
        "[",
        "]",
        "[[]]",
    ]);
    (head, prop::collection::vec(segment, 0..40))
        .prop_map(|(h, segs)| h.to_string() + &segs.concat())
}

fn field_value() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::sample::select(vec![
            "", "0", "1", "-1", "255", "256", "-129", "1e308", "NaN", "on", "off", "true", "admin",
            "user", "Admin", " 42 ", "\u{0}",
        ])
        .prop_map(String::from),
        ".{0,20}",
    ]
}

fn urlencoded(fields: &[(String, String)]) -> Vec<u8> {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish()
        .into_bytes()
}

const BOUNDARY: &str = "fuzzBOUNDARY";

fn multipart(fields: &[(String, String, bool)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value, is_file) in fields {
        // Quotes and newlines would break the part headers themselves.
        let name = name.replace(['"', '\r', '\n'], "");
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        if *is_file {
            body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"f.png\"\r\nContent-Type: image/png\r\n\r\n")
                    .as_bytes(),
            );
        } else {
            body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            );
        }
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_urlencoded_bytes(body in prop::collection::vec(any::<u8>(), 0..2048), pre in any::<bool>()) {
        let status = send("application/x-www-form-urlencoded", body, pre);
        prop_assert!(acceptable(status), "{status}");
    }

    #[test]
    fn structured_urlencoded(fields in prop::collection::vec((field_name(), field_value()), 0..60), pre in any::<bool>()) {
        let status = send("application/x-www-form-urlencoded", urlencoded(&fields), pre);
        prop_assert!(acceptable(status), "{status}");
    }

    #[test]
    fn structured_multipart(
        fields in prop::collection::vec((field_name(), field_value(), any::<bool>()), 0..40),
        pre in any::<bool>(),
    ) {
        let status = send(
            &format!("multipart/form-data; boundary={BOUNDARY}"),
            multipart(&fields),
            pre,
        );
        prop_assert!(acceptable(status), "{status}");
    }

    /// Very deep names: recursion depth must not depend on the input.
    #[test]
    fn deep_names(
        seg in prop::sample::select(vec!["[a]", "[0]", "[]"]),
        depth in 0usize..20_000,
        multi in any::<bool>(),
    ) {
        let name = format!("meta{}", seg.repeat(depth));
        let status = if multi {
            send(
                &format!("multipart/form-data; boundary={BOUNDARY}"),
                multipart(&[(name, "x".into(), false)]),
                false,
            )
        } else {
            send("application/x-www-form-urlencoded", urlencoded(&[(name, "x".into())]), false)
        };
        prop_assert!(acceptable(status), "{status}");
    }

    #[test]
    fn arbitrary_multipart_bytes(body in prop::collection::vec(any::<u8>(), 0..2048)) {
        let status = send(&format!("multipart/form-data; boundary={BOUNDARY}"), body, false);
        prop_assert!(acceptable(status), "{status}");
    }

    #[test]
    fn arbitrary_json(body in prop::collection::vec(any::<u8>(), 0..1024)) {
        let status = send("application/json", body, false);
        prop_assert!(acceptable(status), "{status}");
    }
}

#[test]
fn field_count_is_bounded() {
    let fields = |n: usize| -> Vec<(String, String)> {
        std::iter::once(("name".to_string(), "Ann".to_string()))
            .chain((1..n).map(|i| (format!("meta[k{i}]"), "v".to_string())))
            .collect()
    };
    let ct = "application/x-www-form-urlencoded";
    assert_eq!(
        send(ct, urlencoded(&fields(1000)), false),
        StatusCode::FOUND
    );
    assert_eq!(
        send(ct, urlencoded(&fields(1001)), false),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let parts = |n: usize| -> Vec<(String, String, bool)> {
        fields(n).into_iter().map(|(k, v)| (k, v, false)).collect()
    };
    let ct = format!("multipart/form-data; boundary={BOUNDARY}");
    assert_eq!(send(&ct, multipart(&parts(1000)), false), StatusCode::FOUND);
    assert_eq!(
        send(&ct, multipart(&parts(1001)), false),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
