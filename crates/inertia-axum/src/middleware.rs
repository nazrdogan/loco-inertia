use axum::{
    extract::Request,
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{
    extractor::{is_inertia_request, request_url},
    headers::{X_INERTIA, X_INERTIA_LOCATION, X_INERTIA_VERSION},
    InertiaConfig,
};

/// Protocol-level middleware; use with `axum::middleware::from_fn`.
///
/// - always adds `Vary: X-Inertia`;
/// - answers stale-asset Inertia GETs with `409` + `X-Inertia-Location`;
/// - turns `302` into `303` for Inertia `PUT`/`PATCH`/`DELETE` requests.
///
/// Reads the server version from `Extension<InertiaConfig>`, which must be layered
/// outside this middleware.
pub async fn inertia_middleware(req: Request, next: Next) -> Response {
    let is_inertia = is_inertia_request(req.headers());
    let method = req.method().clone();

    if is_inertia && method == Method::GET {
        if let Some(config) = req.extensions().get::<InertiaConfig>() {
            let client_version = req
                .headers()
                .get(X_INERTIA_VERSION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if client_version != config.version_str() {
                let mut res = version_conflict(&request_url(req.uri()));
                if let Ok(v) = HeaderValue::from_str(config.version_str()) {
                    res.headers_mut().insert(X_INERTIA_VERSION, v);
                }
                return with_vary(res);
            }
        }
    }

    let mut res = next.run(req).await;

    if is_inertia
        && res.status() == StatusCode::FOUND
        && matches!(method, Method::PUT | Method::PATCH | Method::DELETE)
    {
        *res.status_mut() = StatusCode::SEE_OTHER;
    }

    with_vary(res)
}

fn version_conflict(url: &str) -> Response {
    let mut res = StatusCode::CONFLICT.into_response();
    if let Ok(v) = HeaderValue::from_str(url) {
        res.headers_mut().insert(X_INERTIA_LOCATION, v);
    }
    res
}

fn with_vary(mut res: Response) -> Response {
    let already = res
        .headers()
        .get_all(header::VARY)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|v| v.trim().eq_ignore_ascii_case(X_INERTIA));
    if !already {
        res.headers_mut()
            .append(header::VARY, HeaderValue::from_static("X-Inertia"));
    }
    res
}
