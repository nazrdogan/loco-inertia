//! CSRF protection (`csrf` feature) with a signed double-submit cookie, the scheme the
//! Inertia client speaks out of the box: the server sets a signed `XSRF-TOKEN` cookie that
//! JavaScript can read, the client echoes it in `X-XSRF-TOKEN` on every request, and unsafe
//! requests (POST, PUT, PATCH, DELETE, …) must send it back unchanged with a valid signature.
//! Another site can make the browser send the cookie but cannot read it, nor set the header
//! on a cross-origin request. The signature rejects tokens the server never issued (e.g. made
//! up by a sibling subdomain); it does not bind a token to one visitor.

use axum::{
    extract::Request,
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, Key, SameSite, SignedCookieJar};
use subtle::ConstantTimeEq;

/// Status for a missing or wrong token, as in Laravel ("page expired").
pub const CSRF_MISMATCH: u16 = 419;

/// Configuration for [`csrf_middleware`], provided via `Extension<Csrf>`.
#[derive(Clone)]
pub struct Csrf {
    key: Key,
    cookie_name: String,
    header_name: String,
    secure: bool,
    exempt: Vec<String>,
}

impl Csrf {
    /// The token cookie is signed with `key` (its signing half; the flash cookie uses the
    /// encryption half, so one key can serve both).
    pub fn new(key: Key) -> Self {
        Self {
            key,
            cookie_name: "XSRF-TOKEN".to_string(),
            header_name: "x-xsrf-token".to_string(),
            secure: false,
            exempt: Vec::new(),
        }
    }

    /// Mark the cookie `Secure` (HTTPS only). Enable it in production.
    #[must_use]
    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    /// Skip the check for `prefix` and the paths below it (e.g. `/api` or webhooks that
    /// authenticate differently). Matches whole segments: `/api` covers `/api/x`, not `/apiary`.
    #[must_use]
    pub fn exempt(mut self, prefix: impl Into<String>) -> Self {
        self.exempt.push(prefix.into());
        self
    }

    fn is_exempt(&self, path: &str) -> bool {
        self.exempt.iter().any(|p| {
            path.strip_prefix(p.as_str())
                .is_some_and(|rest| rest.is_empty() || p.ends_with('/') || rest.starts_with('/'))
        })
    }

    /// The token cookies with a valid signature, as the client sees them (the signed value).
    ///
    /// Each value is verified on its own: with duplicate cookies (e.g. another path or
    /// domain), a jar would verify one of them while a raw lookup returned another.
    fn valid_tokens(&self, headers: &HeaderMap) -> Vec<String> {
        raw_cookies(headers, &self.cookie_name)
            .filter(|raw| {
                let mut single = HeaderMap::new();
                let Ok(value) = format!("{}={raw}", self.cookie_name).parse() else {
                    return false;
                };
                single.insert(header::COOKIE, value);
                SignedCookieJar::from_headers(&single, self.key.clone())
                    .get(&self.cookie_name)
                    .is_some()
            })
            .map(|raw| percent_decode(&raw))
            .collect()
    }

    fn new_cookie(&self) -> Option<Cookie<'static>> {
        let mut bytes = [0u8; 32];
        if let Err(e) = getrandom::fill(&mut bytes) {
            tracing::error!("cannot generate a CSRF token: {e}");
            return None;
        }
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Some(
            Cookie::build((self.cookie_name.clone(), token))
                .path("/")
                .same_site(SameSite::Lax)
                .secure(self.secure)
                // Readable by JavaScript on purpose: the client copies it into the header.
                .http_only(false)
                .build(),
        )
    }
}

impl std::fmt::Debug for Csrf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Csrf")
            .field("cookie_name", &self.cookie_name)
            .field("header_name", &self.header_name)
            .field("secure", &self.secure)
            .field("exempt", &self.exempt)
            .finish_non_exhaustive()
    }
}

/// Rejects unsafe requests without a matching token with 419, and issues the token cookie
/// to clients that do not have a valid one.
pub async fn csrf_middleware(req: Request, next: Next) -> Response {
    let Some(csrf) = req.extensions().get::<Csrf>().cloned() else {
        return next.run(req).await;
    };
    let tokens = csrf.valid_tokens(req.headers());

    let safe = matches!(
        *req.method(),
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    );
    if !safe && !csrf.is_exempt(req.uri().path()) {
        let sent = req
            .headers()
            .get(csrf.header_name.as_str())
            .and_then(|v| v.to_str().ok())
            .map(percent_decode);
        let ok = sent.is_some_and(|sent| {
            tokens
                .iter()
                .any(|token| token.as_bytes().ct_eq(sent.as_bytes()).into())
        });
        if !ok {
            let status = StatusCode::from_u16(CSRF_MISMATCH).unwrap_or(StatusCode::FORBIDDEN);
            return (status, "CSRF token mismatch").into_response();
        }
    }

    let res = next.run(req).await;
    if !tokens.is_empty() {
        return res;
    }
    match csrf.new_cookie() {
        Some(cookie) => {
            let jar = SignedCookieJar::new(csrf.key.clone()).add(cookie);
            (jar, res).into_response()
        }
        None => res,
    }
}

/// The values of cookies named `name` exactly as the browser sent them.
fn raw_cookies<'a>(headers: &'a HeaderMap, name: &'a str) -> impl Iterator<Item = String> + 'a {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(move |(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

/// Undo percent-encoding. The signed value contains base64 (`+`, `/`, `=`), which the cookie
/// is sent with percent-encoded (`=` → `%3D`); clients read it with `decodeURIComponent`
/// before echoing it, so both sides are compared decoded.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(b) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}
