use std::sync::{Arc, Mutex};

use axum::http::Extensions;

use crate::{IntoProp, Props};

/// Props shared with every page rendered for this request, e.g. the current user.
///
/// Add them from any middleware that runs before the handler:
///
/// ```
/// use axum::{extract::Request, middleware::Next, response::Response};
/// use inertia_axum::SharedProps;
///
/// async fn share(mut req: Request, next: Next) -> Response {
///     SharedProps::of(req.extensions_mut()).insert("app_name", "demo");
///     next.run(req).await
/// }
/// ```
///
/// Page props with the same key win over shared ones.
///
/// Shared props are resolved by the first render of a request (lazy ones run at most once),
/// so a handler that renders twice gets them only the first time; that is logged.
#[derive(Clone, Default)]
pub struct SharedProps(Arc<Mutex<Inner>>);

#[derive(Default)]
struct Inner {
    props: Props,
    /// An earlier render took non-empty shared props.
    taken: bool,
}

impl SharedProps {
    /// The shared props of a request, created on first use.
    pub fn of(extensions: &mut Extensions) -> &mut Self {
        extensions.get_or_insert_default::<Self>()
    }

    /// Share `prop` under `key`, replacing an earlier one.
    pub fn insert(&self, key: impl Into<String>, prop: impl IntoProp) {
        self.lock().props.insert(key, prop);
    }

    /// Share every field of a `Serialize` struct (e.g. one typed for the frontend).
    pub fn extend(&self, value: impl serde::Serialize) -> Result<(), crate::BoxError> {
        let props = crate::Props::from_serialize(value)?;
        let mut shared = self.lock();
        shared.props = std::mem::take(&mut shared.props).merge(props);
        Ok(())
    }

    /// Take the props out, leaving this empty (they are resolved once per render).
    pub(crate) fn take(&self) -> Props {
        let mut inner = self.lock();
        if inner.taken {
            tracing::warn!(
                "shared props were already used by an earlier render of this request; \
                 this page only gets the ones shared since"
            );
        }
        let props = std::mem::take(&mut inner.props);
        inner.taken |= !props.is_empty();
        props
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl std::fmt::Debug for SharedProps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SharedProps")
            .field(&self.lock().props)
            .finish()
    }
}
