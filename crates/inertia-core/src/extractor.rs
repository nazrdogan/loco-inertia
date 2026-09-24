use axum::{
    extract::FromRequestParts,
    http::{request::Parts, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
};

use serde::Serialize;
use serde_json::Value;

use crate::{
    flash::{FlashConsumed, FlashData, IncomingFlash, InertiaRedirect},
    headers::{
        PRECOGNITION, PRECOGNITION_SUCCESS, PRECOGNITION_VALIDATE_ONLY, X_INERTIA,
        X_INERTIA_ERROR_BAG, X_INERTIA_EXCEPT_ONCE_PROPS, X_INERTIA_INFINITE_SCROLL_MERGE_INTENT,
        X_INERTIA_LOCATION, X_INERTIA_PARTIAL_COMPONENT, X_INERTIA_PARTIAL_DATA,
        X_INERTIA_PARTIAL_EXCEPT, X_INERTIA_RESET,
    },
    page::{mount_markup, to_script_json},
    BoxError, InertiaConfig, InertiaPage, IntoProps, MergeIntent, Page, Prop, PropRequest, Props,
    RootView, SharedProps, SsrResponse,
};

/// Per-request Inertia context. Extract it in a handler and call [`Inertia::render`].
#[derive(Debug, Clone)]
pub struct Inertia {
    config: InertiaConfig,
    is_inertia: bool,
    url: String,
    partial: Option<PartialHeaders>,
    error_bag: Option<String>,
    /// Where `back()` may go: the `Referer` when it is on this site, as a relative URL.
    back_location: Option<String>,
    shared: Option<SharedProps>,
    /// Flashed by the previous request, plus anything added with [`Inertia::flash`].
    flash: FlashData,
    encrypt_history: Option<bool>,
    except_once: Vec<String>,
    /// `Some(validate_only)` for a Precognition (live validation) request.
    precognition: Option<Vec<String>>,
}

/// The raw partial reload headers; they only apply when the component matches.
#[derive(Debug, Clone)]
struct PartialHeaders {
    component: String,
    only: Vec<String>,
    except: Vec<String>,
    reset: Vec<String>,
    merge_intent: MergeIntent,
}

/// Returned when the handler is not wrapped in `Extension<InertiaConfig>`.
#[derive(Debug)]
pub struct InertiaRejection;

impl IntoResponse for InertiaRejection {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Inertia is not configured: add `Extension(InertiaConfig)` to the router",
        )
            .into_response()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Inertia {
    type Rejection = InertiaRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let config = parts
            .extensions
            .get::<InertiaConfig>()
            .cloned()
            .ok_or(InertiaRejection)?;
        let is_inertia = is_inertia_request(&parts.headers);
        let partial = if is_inertia {
            header_str(&parts.headers, X_INERTIA_PARTIAL_COMPONENT).map(|component| {
                PartialHeaders {
                    component: component.to_string(),
                    only: header_list(&parts.headers, X_INERTIA_PARTIAL_DATA),
                    except: header_list(&parts.headers, X_INERTIA_PARTIAL_EXCEPT),
                    reset: header_list(&parts.headers, X_INERTIA_RESET),
                    merge_intent: MergeIntent::from_header(header_str(
                        &parts.headers,
                        X_INERTIA_INFINITE_SCROLL_MERGE_INTENT,
                    )),
                }
            })
        } else {
            None
        };
        let back_location = header_str(&parts.headers, "referer").and_then(|referer| {
            same_site_location(
                referer,
                header_str(&parts.headers, "host"),
                &config.allowed_redirect_hosts,
            )
        });
        Ok(Self {
            config,
            is_inertia,
            url: request_url(&parts.uri),
            partial,
            error_bag: header_str(&parts.headers, X_INERTIA_ERROR_BAG)
                .filter(|b| !b.is_empty())
                .map(ToString::to_string),
            back_location,
            shared: parts.extensions.get::<SharedProps>().cloned(),
            flash: parts
                .extensions
                .get::<IncomingFlash>()
                .map(|f| f.0.clone())
                .unwrap_or_default(),
            encrypt_history: None,
            precognition: header_str(&parts.headers, PRECOGNITION)
                .filter(|v| v.eq_ignore_ascii_case("true"))
                .map(|_| header_list(&parts.headers, PRECOGNITION_VALIDATE_ONLY)),
            except_once: if is_inertia {
                header_list(&parts.headers, X_INERTIA_EXCEPT_ONCE_PROPS)
            } else {
                Vec::new()
            },
        })
    }
}

impl Inertia {
    /// Whether the current request was made by the Inertia client.
    pub fn is_inertia_request(&self) -> bool {
        self.is_inertia
    }

    /// What the client asked for when rendering `component`.
    pub fn prop_request(&self, component: &str) -> PropRequest {
        let mut req = match &self.partial {
            Some(p) if p.component == component => PropRequest {
                partial: true,
                only: p.only.clone(),
                except: p.except.clone(),
                reset: p.reset.clone(),
                merge_intent: p.merge_intent,
                except_once: Vec::new(),
            },
            _ => PropRequest::default(),
        };
        req.except_once.clone_from(&self.except_once);
        req
    }

    /// Add one-time data to the page rendered by this request (`page.flash`).
    #[must_use]
    pub fn flash(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        match serde_json::to_value(value) {
            Ok(v) => {
                self.flash.flash.insert(key.into(), v);
            }
            Err(e) => tracing::error!("invalid flash value: {e}"),
        }
        self
    }

    /// Whether this is a Precognition request: the client (`useForm().validate()`) only
    /// wants the submitted data validated; the action itself must not run.
    pub fn is_precognitive(&self) -> bool {
        self.precognition.is_some()
    }

    /// The answer to a Precognition request given the validation errors of the submitted
    /// data (field → message or messages): `204` with `Precognition-Success` when there are
    /// none for the fields the client asked about (`Precognition-Validate-Only`), else `422`
    /// with `{ "errors": ... }`.
    pub fn precognition_response(&self, errors: impl Serialize) -> Response {
        let errors = match crate::flash::to_object(errors) {
            Ok(errors) => errors,
            Err(e) => return server_error(&format!("invalid validation errors: {e}")),
        };
        let only = self.precognition.as_deref().unwrap_or_default();
        let errors: serde_json::Map<String, Value> = errors
            .into_iter()
            .filter(|(field, _)| {
                only.is_empty()
                    || only.iter().any(|o| {
                        o == field
                            || field
                                .strip_prefix(o.as_str())
                                .is_some_and(|r| r.starts_with('.'))
                    })
            })
            .collect();
        let mut res = if errors.is_empty() {
            let mut res = StatusCode::NO_CONTENT.into_response();
            res.headers_mut()
                .insert(PRECOGNITION_SUCCESS, HeaderValue::from_static("true"));
            res
        } else {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(serde_json::json!({
                    "message": "The given data was invalid.",
                    "errors": errors,
                })),
            )
                .into_response()
        };
        res.headers_mut()
            .insert(PRECOGNITION, HeaderValue::from_static("true"));
        res.headers_mut().append(
            axum::http::header::VARY,
            HeaderValue::from_static("Precognition"),
        );
        res
    }

    /// Encrypt (or not) this page in the client's history, overriding the config default.
    #[must_use]
    pub fn encrypt_history(mut self, encrypt: bool) -> Self {
        self.encrypt_history = Some(encrypt);
        self
    }

    /// Clear the client's history when this page arrives; to do it after a redirect use
    /// [`InertiaRedirect::clear_history`].
    #[must_use]
    pub fn clear_history(mut self) -> Self {
        self.flash.clear_history = true;
        self
    }

    /// Redirect to `location`; attach errors or flash data with the returned builder.
    pub fn redirect(&self, location: impl Into<String>) -> InertiaRedirect {
        InertiaRedirect::new(location.into(), self.error_bag.clone())
    }

    /// Redirect to the previous page (the `Referer`), or `/` when there is none or it points
    /// to another site.
    pub fn back(&self) -> InertiaRedirect {
        self.back_or("/")
    }

    /// Like [`Inertia::back`] with another fallback.
    pub fn back_or(&self, fallback: impl Into<String>) -> InertiaRedirect {
        let location = self
            .back_location
            .clone()
            .unwrap_or_else(|| fallback.into());
        self.redirect(location)
    }

    /// A full page visit to `url`, e.g. an external site: `409` with `X-Inertia-Location`
    /// for Inertia requests (XHR cannot follow those), a plain redirect otherwise.
    pub fn location(&self, url: &str) -> Response {
        let Ok(value) = HeaderValue::from_str(url) else {
            return server_error(&format!("invalid location {url:?}"));
        };
        if self.is_inertia {
            (StatusCode::CONFLICT, [(X_INERTIA_LOCATION, value)]).into_response()
        } else {
            (StatusCode::FOUND, [(axum::http::header::LOCATION, value)]).into_response()
        }
    }

    /// Render `component` with `props`: JSON for Inertia requests, the root HTML otherwise.
    ///
    /// `props` is either a [`crate::Props`] tree or any `Serialize` value that serializes
    /// to a JSON object. Lazy props are resolved here, only if they are part of the response.
    /// Shared props are merged underneath, and `errors` is always present.
    pub async fn render(&self, component: &str, props: impl IntoProps) -> Response {
        match self.build_page(component, props).await {
            Ok(page) => {
                let mut res = self.render_page(&page).await;
                res.extensions_mut().insert(FlashConsumed);
                res
            }
            Err(e) => server_error(&format!("failed to render Inertia page `{component}`: {e}")),
        }
    }

    /// Render the page component that `props` belongs to.
    pub async fn page<P: InertiaPage + IntoProps>(&self, props: P) -> Response {
        self.render(P::COMPONENT, props).await
    }

    /// Like [`Inertia::page`], adding lazy props (defer, optional, …) to the typed ones.
    /// Declare those fields in the props type as skipped-when-`None` options so the
    /// frontend type knows them: `#[serde(skip_serializing_if = "Option::is_none")]`.
    pub async fn page_with<P: InertiaPage + serde::Serialize>(
        &self,
        props: P,
        extend: impl FnOnce(Props) -> Props,
    ) -> Response {
        match Props::from_serialize(props) {
            Ok(props) => self.render(P::COMPONENT, extend(props)).await,
            Err(e) => server_error(&format!(
                "failed to render Inertia page `{}`: {e}",
                P::COMPONENT
            )),
        }
    }

    /// Resolve props and build the page object without rendering it.
    pub async fn build_page(
        &self,
        component: &str,
        props: impl IntoProps,
    ) -> Result<Page, BoxError> {
        let page_props = props.into_props()?;
        let shared = self
            .shared
            .as_ref()
            .map(SharedProps::take)
            .unwrap_or_default();
        let shared_keys: Vec<String> = shared
            .keys()
            .filter(|k| !page_props.contains_key(k))
            .map(ToString::to_string)
            .collect();
        let mut props = shared.merge(page_props);
        if !props.contains_key("errors") {
            props.insert(
                "errors",
                Prop::always(Value::Object(self.flash.errors.clone())),
            );
        }

        let resolved = props.resolve(&self.prop_request(component)).await?;
        let mut page = Page::new(
            component,
            serde_json::Value::Object(resolved.props),
            self.url.clone(),
            self.config.version_str(),
        );
        page.merge_props = resolved.merge_props;
        page.prepend_props = resolved.prepend_props;
        page.deep_merge_props = resolved.deep_merge_props;
        page.match_props_on = resolved.match_props_on;
        page.deferred_props = resolved.deferred_props;
        page.scroll_props = resolved.scroll_props;
        page.once_props = resolved.once_props;
        page.shared_props = shared_keys;
        page.flash = self.flash.flash.clone();
        page.encrypt_history = self.encrypt_history.unwrap_or(self.config.encrypt_history);
        page.clear_history = self.flash.clear_history;
        Ok(page)
    }

    /// Render an already built page object.
    pub async fn render_page(&self, page: &Page) -> Response {
        if self.is_inertia {
            let mut res = axum::Json(page).into_response();
            res.headers_mut()
                .insert(X_INERTIA, HeaderValue::from_static("true"));
            return res;
        }

        let (head, body) = match self.server_render(page).await {
            Some(ssr) => (ssr.head.join("\n"), ssr.body),
            None => match to_script_json(page) {
                Ok(json) => (String::new(), mount_markup(&self.config.app_id, &json)),
                Err(e) => return server_error(&format!("failed to serialize Inertia page: {e}")),
            },
        };
        match (self.config.root_template)(&RootView {
            page,
            body: &body,
            head: &head,
        }) {
            Ok(html) => Html(html).into_response(),
            Err(e) => server_error(&format!("failed to render Inertia root template: {e}")),
        }
    }
}

impl Inertia {
    /// SSR output for a first visit, or `None` to render client-side (no renderer
    /// configured, or it failed).
    async fn server_render(&self, page: &Page) -> Option<SsrResponse> {
        let ssr = self.config.ssr.as_ref()?;
        match ssr.render(page).await {
            Ok(res) => Some(res),
            Err(e) => {
                tracing::warn!(
                    component = page.component,
                    "SSR failed, rendering client-side: {e}"
                );
                None
            }
        }
    }
}

/// The `Referer` as a relative URL (`/path?query`) if it belongs to this site, so `back()`
/// can never become an open redirect (or send the XHR to a host it cannot follow).
///
/// Absolute referers must be http(s) with the request's `Host` or one of `allowed_hosts`
/// (for proxies that rewrite `Host`); relative ones must be a plain path, not `//host` or
/// `/\host`, which browsers treat as another site.
pub(crate) fn same_site_location(
    referer: &str,
    host: Option<&str>,
    allowed_hosts: &[String],
) -> Option<String> {
    let uri: axum::http::Uri = referer.parse().ok()?;
    let path = uri.path_and_query().map_or("/", |pq| pq.as_str());
    let safe_path = |p: &str| p.starts_with('/') && !p.starts_with("//") && !p.starts_with("/\\");
    match uri.authority() {
        None => (uri.scheme().is_none() && safe_path(referer)).then(|| referer.to_string()),
        Some(authority) => {
            let scheme_ok = matches!(uri.scheme_str(), Some("http" | "https"));
            let same_host = host
                .into_iter()
                .chain(allowed_hosts.iter().map(String::as_str))
                .any(|h| h.eq_ignore_ascii_case(authority.as_str()));
            (scheme_ok && same_host && safe_path(path)).then(|| path.to_string())
        }
    }
}

pub(crate) fn is_inertia_request(headers: &HeaderMap) -> bool {
    headers.contains_key(X_INERTIA)
}

/// Path and query of the request, e.g. `/users?page=2`.
pub(crate) fn request_url(uri: &axum::http::Uri) -> String {
    uri.path_and_query()
        .map_or_else(|| uri.path().to_string(), |pq| pq.as_str().to_string())
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// A comma separated header as a list, ignoring blanks.
fn header_list(headers: &HeaderMap, name: &str) -> Vec<String> {
    header_str(headers, name)
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Log the details, answer with a generic 500 so internals do not leak to clients.
fn server_error(msg: &str) -> Response {
    tracing::error!("{msg}");
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
}
