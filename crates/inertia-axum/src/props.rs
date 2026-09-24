//! The prop tree: eager values, lazily resolved props and nested groups, each with an
//! inclusion rule (default / optional / always / defer) and optional merge behaviour.

use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{BoxError, MergeIntent, ScrollMeta};

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type Resolver = Box<dyn FnOnce() -> BoxFuture<Result<Value, BoxError>> + Send>;
type LevelFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Map<String, Value>, BoxError>> + Send + 'a>>;

/// A group of named props. Keys may themselves hold nested [`Props`].
///
/// ```
/// use inertia_axum::{Prop, Props};
///
/// let props = Props::new()
///     .with("user", serde_json::json!({ "name": "Ann" }))
///     .with("stats", Prop::defer(|| async { Ok::<_, std::convert::Infallible>(42) }))
///     .with("sidebar", Props::new().with("links", Prop::optional(|| async {
///         Ok::<_, std::convert::Infallible>(vec!["a", "b"])
///     })));
/// ```
#[derive(Default)]
pub struct Props {
    entries: Vec<(String, Prop)>,
}

impl Props {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add (or replace) the prop at `key`.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, prop: impl IntoProp) -> Self {
        self.insert(key, prop);
        self
    }

    /// Add (or replace) the prop at `key`.
    pub fn insert(&mut self, key: impl Into<String>, prop: impl IntoProp) {
        let key = key.into();
        let prop = prop.into_prop();
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = prop,
            None => self.entries.push((key, prop)),
        }
    }

    /// Merge `other` into `self`; keys in `other` win.
    #[must_use]
    pub fn merge(mut self, other: Props) -> Self {
        for (key, prop) in other.entries {
            self.insert(key, prop);
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// Turn the prop at `key` into an infinite scroll prop (see [`Prop::scroll`]), e.g.
    /// after building props from a typed struct whose field is a [`crate::ScrollData`].
    #[must_use]
    pub fn scroll(mut self, key: &str, meta: ScrollMeta) -> Self {
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some((_, prop)) => prop.scroll = Some(meta),
            None => tracing::error!("cannot make `{key}` a scroll prop: there is no such prop"),
        }
        self
    }

    /// Top-level keys, in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }
}

impl fmt::Debug for Props {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.entries.iter().map(|(k, v)| (k, v)))
            .finish()
    }
}

/// Anything accepted as the props of a page: a [`Props`] tree or any `Serialize` value that
/// serializes to a JSON object (or `null`/`()` for no props).
pub trait IntoProps {
    fn into_props(self) -> Result<Props, BoxError>;
}

impl Props {
    /// Props from a `Serialize` value that serializes to a JSON object; add lazy props
    /// with [`Props::with`] afterwards.
    pub fn from_serialize(value: impl Serialize) -> Result<Props, BoxError> {
        value.into_props()
    }
}

/// A props type that knows which page component it belongs to, so handlers can call
/// `inertia.page(props)` and the component name cannot drift from the props type.
///
/// ```
/// #[derive(serde::Serialize)]
/// struct HomeProps { greeting: String }
///
/// impl inertia_axum::InertiaPage for HomeProps {
///     const COMPONENT: &'static str = "Home";
/// }
/// ```
pub trait InertiaPage {
    const COMPONENT: &'static str;
}

impl IntoProps for Props {
    fn into_props(self) -> Result<Props, BoxError> {
        Ok(self)
    }
}

impl<T: Serialize> IntoProps for T {
    fn into_props(self) -> Result<Props, BoxError> {
        match serde_json::to_value(self)? {
            Value::Null => Ok(Props::new()),
            Value::Object(map) => Ok(Props {
                entries: map.into_iter().map(|(k, v)| (k, Prop::value(v))).collect(),
            }),
            other => {
                Err(format!("Inertia props must serialize to a JSON object, got {other}").into())
            }
        }
    }
}

/// Anything that can sit at a key of [`Props`].
pub trait IntoProp {
    fn into_prop(self) -> Prop;
}

impl IntoProp for Prop {
    fn into_prop(self) -> Prop {
        self
    }
}

impl IntoProp for Props {
    fn into_prop(self) -> Prop {
        Prop::new(Source::Nested(self), Kind::Default)
    }
}

impl<T: Serialize> IntoProp for T {
    fn into_prop(self) -> Prop {
        Prop::value(self)
    }
}

/// A single prop: where its value comes from and when it is sent.
pub struct Prop {
    source: Source,
    kind: Kind,
    merge: Option<Merge>,
    scroll: Option<ScrollMeta>,
    once: Option<Once>,
}

/// Resolve-once settings: the client keeps the value and tells the server which ones it
/// has (`X-Inertia-Except-Once-Props`), so they are not recomputed on later visits.
#[derive(Debug, Clone, Default)]
struct Once {
    key: Option<String>,
    expiry: Option<Expiry>,
    fresh: bool,
}

#[derive(Debug, Clone, Copy)]
enum Expiry {
    In(Duration),
    At(SystemTime),
}

enum Source {
    Value(Result<Value, String>),
    Lazy(Resolver),
    Nested(Props),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// Sent on full visits and when requested by a partial reload.
    Default,
    /// Never sent on full visits; only when a partial reload asks for it.
    Optional,
    /// Always sent, even when a partial reload does not ask for it.
    Always,
    /// Omitted and announced on full visits; the client fetches each group afterwards.
    Defer { group: String },
}

#[derive(Debug, Clone)]
struct Merge {
    strategy: MergeStrategy,
    match_on: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum MergeStrategy {
    Append,
    Prepend,
    Deep,
}

impl Prop {
    fn new(source: Source, kind: Kind) -> Self {
        Self {
            source,
            kind,
            merge: None,
            scroll: None,
            once: None,
        }
    }

    fn resolver<F, Fut, T, E>(f: F) -> Source
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: Serialize,
        E: Into<BoxError>,
    {
        Source::Lazy(Box::new(move || {
            Box::pin(async move {
                let value = f().await.map_err(Into::into)?;
                Ok(serde_json::to_value(value)?)
            })
        }))
    }

    /// An eagerly serialized value.
    pub fn value(value: impl Serialize) -> Self {
        let value = serde_json::to_value(value).map_err(|e| e.to_string());
        Self::new(Source::Value(value), Kind::Default)
    }

    /// A normal prop whose value is only computed when it is part of the response.
    pub fn lazy<F, Fut, T, E>(f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: Serialize,
        E: Into<BoxError>,
    {
        Self::new(Self::resolver(f), Kind::Default)
    }

    /// Only sent when a partial reload explicitly requests it.
    pub fn optional<F, Fut, T, E>(f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: Serialize,
        E: Into<BoxError>,
    {
        Self::new(Self::resolver(f), Kind::Optional)
    }

    /// Loaded by the client right after the first render, in the `default` group.
    pub fn defer<F, Fut, T, E>(f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        T: Serialize,
        E: Into<BoxError>,
    {
        Self::new(
            Self::resolver(f),
            Kind::Defer {
                group: "default".to_string(),
            },
        )
    }

    /// Included in every response, ignoring partial reload filters.
    pub fn always(value: impl IntoProp) -> Self {
        let mut prop = value.into_prop();
        prop.kind = Kind::Always;
        prop
    }

    /// Put a deferred prop in a named group (deferred props of one group load together).
    #[must_use]
    pub fn group(mut self, group: impl Into<String>) -> Self {
        if let Kind::Defer { group: g } = &mut self.kind {
            *g = group.into();
        }
        self
    }

    /// The client appends arrays / shallow-merges objects instead of replacing.
    #[must_use]
    pub fn merge(self) -> Self {
        self.with_strategy(MergeStrategy::Append)
    }

    /// The client prepends to arrays instead of replacing.
    #[must_use]
    pub fn prepend(self) -> Self {
        self.with_strategy(MergeStrategy::Prepend)
    }

    /// The client merges objects recursively instead of replacing.
    #[must_use]
    pub fn deep_merge(self) -> Self {
        self.with_strategy(MergeStrategy::Deep)
    }

    /// When merging, items with the same `key` field are replaced instead of duplicated.
    /// Implies [`Prop::merge`] if no merge strategy was chosen.
    #[must_use]
    pub fn match_on(mut self, key: impl Into<String>) -> Self {
        let merge = self.merge.get_or_insert(Merge {
            strategy: MergeStrategy::Append,
            match_on: Vec::new(),
        });
        merge.match_on.push(key.into());
        self
    }

    /// Make this an infinite scroll prop: its value holds the items under the meta's
    /// wrapper key (`{ "data": [...] }`, see [`crate::ScrollData`]); each loaded page is
    /// appended or prepended, as the client asks, and `scrollProps` carries `meta`.
    /// Combine with [`Prop::match_on`] to de-duplicate items.
    #[must_use]
    pub fn scroll(mut self, meta: ScrollMeta) -> Self {
        self.scroll = Some(meta);
        self
    }

    /// Resolve once and let the client reuse the value on later visits (to any page sending
    /// a once prop with the same key) instead of recomputing it. Keyed by the prop path.
    #[must_use]
    pub fn once(mut self) -> Self {
        self.once.get_or_insert_with(Once::default);
        self
    }

    /// Like [`Prop::once`] with an explicit key, to share the value across pages that put
    /// it under different paths.
    #[must_use]
    pub fn once_as(mut self, key: impl Into<String>) -> Self {
        self.once.get_or_insert_with(Once::default).key = Some(key.into());
        self
    }

    /// A once prop the client reloads after this long.
    #[must_use]
    pub fn expires_in(mut self, ttl: Duration) -> Self {
        self.once.get_or_insert_with(Once::default).expiry = Some(Expiry::In(ttl));
        self
    }

    /// A once prop the client reloads after this time.
    #[must_use]
    pub fn expires_at(mut self, at: SystemTime) -> Self {
        self.once.get_or_insert_with(Once::default).expiry = Some(Expiry::At(at));
        self
    }

    /// Resolve this once prop on this response even if the client already has it (e.g.
    /// after the underlying data changed); the client stores the new value.
    #[must_use]
    pub fn fresh(mut self, fresh: bool) -> Self {
        self.once.get_or_insert_with(Once::default).fresh = fresh;
        self
    }

    fn with_strategy(mut self, strategy: MergeStrategy) -> Self {
        match &mut self.merge {
            Some(m) => m.strategy = strategy,
            None => {
                self.merge = Some(Merge {
                    strategy,
                    match_on: Vec::new(),
                });
            }
        }
        self
    }
}

impl fmt::Debug for Prop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Prop");
        match &self.source {
            Source::Value(v) => d.field("value", v),
            Source::Lazy(_) => d.field("value", &"<lazy>"),
            Source::Nested(p) => d.field("props", p),
        };
        d.field("kind", &self.kind)
            .field("merge", &self.merge)
            .finish()
    }
}

/// What the client asked for, parsed from the partial reload headers.
#[derive(Debug, Default, Clone)]
pub struct PropRequest {
    /// The request is a partial reload of this very component.
    pub partial: bool,
    /// `X-Inertia-Partial-Data`.
    pub only: Vec<String>,
    /// `X-Inertia-Partial-Except`.
    pub except: Vec<String>,
    /// `X-Inertia-Reset`.
    pub reset: Vec<String>,
    /// `X-Inertia-Infinite-Scroll-Merge-Intent`.
    pub merge_intent: MergeIntent,
    /// `X-Inertia-Except-Once-Props`: once props the client already has (any Inertia
    /// request, not only partial reloads).
    pub except_once: Vec<String>,
}

/// Props resolved for one response plus the page metadata they produce.
#[derive(Debug, Default)]
pub struct Resolved {
    pub props: Map<String, Value>,
    pub merge_props: Vec<String>,
    pub prepend_props: Vec<String>,
    pub deep_merge_props: Vec<String>,
    pub match_props_on: Vec<String>,
    pub deferred_props: BTreeMap<String, Vec<String>>,
    /// Pagination metadata of the infinite scroll props in the response, by path.
    pub scroll_props: BTreeMap<String, Value>,
    /// Once props in the response (or skipped because the client has them), by key.
    pub once_props: BTreeMap<String, Value>,
}

impl Props {
    /// Resolve the tree for a request: filter by kind and partial reload headers, run lazy
    /// resolvers for the props that are sent, and collect merge/defer metadata.
    pub async fn resolve(self, req: &PropRequest) -> Result<Resolved, BoxError> {
        let mut out = Resolved::default();
        out.props = resolve_level(self, "", false, req, &mut out).await?;
        Ok(out)
    }
}

fn resolve_level<'a>(
    props: Props,
    prefix: &'a str,
    ancestor_requested: bool,
    req: &'a PropRequest,
    out: &'a mut Resolved,
) -> LevelFuture<'a> {
    Box::pin(async move {
        // Siblings resolve concurrently (lazy props usually wait on I/O); each collects its
        // metadata separately, merged afterwards in key order so the output is deterministic.
        let siblings = props.entries.into_iter().map(|(key, prop)| async move {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            let mut meta = Resolved::default();
            let value = resolve_prop(prop, &path, ancestor_requested, req, &mut meta).await?;
            Ok::<_, BoxError>((key, value, meta))
        });
        let mut map = Map::new();
        for (key, value, meta) in futures_util::future::try_join_all(siblings).await? {
            out.absorb(meta);
            if let Some(value) = value {
                map.insert(key, value);
            }
        }
        Ok(map)
    })
}

impl Resolved {
    /// Add the metadata (not the props) collected for a subtree.
    fn absorb(&mut self, other: Resolved) {
        self.merge_props.extend(other.merge_props);
        self.prepend_props.extend(other.prepend_props);
        self.deep_merge_props.extend(other.deep_merge_props);
        self.match_props_on.extend(other.match_props_on);
        for (group, paths) in other.deferred_props {
            self.deferred_props.entry(group).or_default().extend(paths);
        }
        self.scroll_props.extend(other.scroll_props);
        self.once_props.extend(other.once_props);
    }
}

async fn resolve_prop(
    prop: Prop,
    path: &str,
    ancestor_requested: bool,
    req: &PropRequest,
    out: &mut Resolved,
) -> Result<Option<Value>, BoxError> {
    // What to do with this node: include it whole, include it but narrow it down to
    // descendant paths, or leave it out.
    let requested_descendants: Vec<&str>;
    let whole: bool;

    if !req.partial {
        match &prop.kind {
            Kind::Optional => return Ok(None),
            Kind::Defer { group } => {
                if let Some(once) = &prop.once {
                    let (key, meta) = once_meta(once, path);
                    if !once.fresh && req.except_once.contains(&key) {
                        // The client already has the value: nothing to load after the render.
                        out.once_props.insert(key, meta);
                        return Ok(None);
                    }
                }
                out.deferred_props
                    .entry(group.clone())
                    .or_default()
                    .push(path.to_string());
                return Ok(None);
            }
            Kind::Default | Kind::Always => {}
        }
        whole = true;
        requested_descendants = Vec::new();
    } else if prop.kind == Kind::Always {
        whole = true;
        requested_descendants = Vec::new();
    } else {
        if req.except.iter().any(|e| covers(e, path)) {
            return Ok(None);
        }
        whole = ancestor_requested || req.only.is_empty() || req.only.iter().any(|o| o == path);
        requested_descendants = if whole {
            Vec::new()
        } else {
            req.only
                .iter()
                .filter_map(|o| strip_parent(o, path))
                .collect()
        };
        if !whole && requested_descendants.is_empty() {
            return Ok(None);
        }
    }

    if let Some(once) = &prop.once {
        let (key, meta) = once_meta(once, path);
        out.once_props.insert(key.clone(), meta);
        // An explicit partial reload of the prop (e.g. `router.reload({ only: [...] })`)
        // asks for a new value.
        let requested = req.partial && req.only.iter().any(|o| o == path);
        if !once.fresh && !requested && req.except_once.contains(&key) {
            return Ok(None);
        }
    }

    let value = match prop.source {
        // Children apply the partial reload filters themselves; they count as requested
        // when this node was requested as a whole.
        Source::Nested(children) => {
            let requested = whole && req.partial;
            Value::Object(resolve_level(children, path, requested, req, out).await?)
        }
        Source::Value(v) => {
            let v = v.map_err(|e| format!("failed to serialize prop `{path}`: {e}"))?;
            narrow_leaf(v, path, &requested_descendants, &prop.kind, req)
        }
        Source::Lazy(f) => {
            let v = f()
                .await
                .map_err(|e| format!("failed to resolve prop `{path}`: {e}"))?;
            narrow_leaf(v, path, &requested_descendants, &prop.kind, req)
        }
    };

    let reset = req.reset.iter().any(|r| r == path);
    if let Some(meta) = &prop.scroll {
        if !reset {
            let items = format!("{path}.{}", meta.wrapper);
            match req.merge_intent {
                MergeIntent::Append => out.merge_props.push(items.clone()),
                MergeIntent::Prepend => out.prepend_props.push(items.clone()),
            }
            let prop_keys = prop.merge.iter().flat_map(|m| m.match_on.iter());
            out.match_props_on.extend(
                meta.match_on
                    .iter()
                    .chain(prop_keys)
                    .map(|k| format!("{items}.{k}")),
            );
        }
        out.scroll_props
            .insert(path.to_string(), meta.to_json(reset));
    } else if let Some(merge) = &prop.merge {
        if !reset {
            let list = match merge.strategy {
                MergeStrategy::Append => &mut out.merge_props,
                MergeStrategy::Prepend => &mut out.prepend_props,
                MergeStrategy::Deep => &mut out.deep_merge_props,
            };
            list.push(path.to_string());
            out.match_props_on
                .extend(merge.match_on.iter().map(|k| format!("{path}.{k}")));
        }
    }

    Ok(Some(value))
}

/// The key of a once prop and its `onceProps` entry: `{ prop, expiresAt }`.
fn once_meta(once: &Once, path: &str) -> (String, Value) {
    let key = once.key.clone().unwrap_or_else(|| path.to_string());
    let expires_at = once.expiry.map(|e| {
        let at = match e {
            Expiry::In(ttl) => SystemTime::now() + ttl,
            Expiry::At(at) => at,
        };
        at.duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    });
    (
        key,
        serde_json::json!({ "prop": path, "expiresAt": expires_at }),
    )
}

/// Apply dot paths that point *inside* a plain (non-[`Props`]) value.
fn narrow_leaf(
    mut value: Value,
    path: &str,
    requested_descendants: &[&str],
    kind: &Kind,
    req: &PropRequest,
) -> Value {
    if !req.partial || *kind == Kind::Always {
        return value;
    }
    if !requested_descendants.is_empty() {
        value = keep_paths(value, requested_descendants);
    }
    let excluded: Vec<&str> = req
        .except
        .iter()
        .filter_map(|e| strip_parent(e, path))
        .collect();
    if excluded.is_empty() {
        value
    } else {
        remove_paths(value, &excluded)
    }
}

/// `pattern` names `path` itself or one of its ancestors.
fn covers(pattern: &str, path: &str) -> bool {
    pattern == path || strip_parent(path, pattern).is_some()
}

/// If `path` is strictly below `parent`, the remainder (`a.b.c` under `a` → `b.c`).
fn strip_parent<'p>(path: &'p str, parent: &str) -> Option<&'p str> {
    path.strip_prefix(parent)?.strip_prefix('.')
}

/// Keep only the given dot paths of a resolved JSON value.
fn keep_paths(value: Value, paths: &[&str]) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    let mut kept = Map::new();
    let mut heads: Vec<&str> = paths.iter().map(|p| head(p).0).collect();
    heads.sort_unstable();
    heads.dedup();
    for key in heads {
        let Some(child) = map.remove(key) else {
            continue;
        };
        let rest: Vec<Option<&str>> = paths
            .iter()
            .filter_map(|p| match head(p) {
                (h, rest) if h == key => Some(rest),
                _ => None,
            })
            .collect();
        let child = if rest.iter().any(Option::is_none) {
            child
        } else {
            keep_paths(child, &rest.into_iter().flatten().collect::<Vec<_>>())
        };
        kept.insert(key.to_string(), child);
    }
    Value::Object(kept)
}

/// Remove the given dot paths from a resolved JSON value.
fn remove_paths(value: Value, paths: &[&str]) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    for path in paths {
        match head(path) {
            (key, None) => {
                map.remove(key);
            }
            (key, Some(rest)) => {
                if let Some(child) = map.remove(key) {
                    map.insert(key.to_string(), remove_paths(child, &[rest]));
                }
            }
        }
    }
    Value::Object(map)
}

fn head(path: &str) -> (&str, Option<&str>) {
    match path.split_once('.') {
        Some((h, rest)) => (h, Some(rest)),
        None => (path, None),
    }
}
