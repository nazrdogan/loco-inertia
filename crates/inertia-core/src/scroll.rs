//! Infinite scroll (`<InfiniteScroll data="posts">`): a prop whose items live under a
//! wrapper key (`posts.data`) and are merged page by page, plus the pagination metadata the
//! client keeps in `scrollProps`.

use serde::Serialize;
use serde_json::Value;

/// The wire shape of a scroll prop with the default wrapper: `{ "data": [...] }`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScrollData<T> {
    pub data: Vec<T>,
}

impl<T> ScrollData<T> {
    pub fn new(data: Vec<T>) -> Self {
        Self { data }
    }
}

/// Where the client is in the pagination. Pages are numbers or opaque cursors.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollMeta {
    pub(crate) page_name: String,
    pub(crate) current_page: Value,
    pub(crate) previous_page: Value,
    pub(crate) next_page: Value,
    pub(crate) wrapper: String,
    pub(crate) match_on: Vec<String>,
}

/// Whether the client wants the loaded page after (`append`) or before (`prepend`) the
/// items it already has: `X-Inertia-Infinite-Scroll-Merge-Intent`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MergeIntent {
    #[default]
    Append,
    Prepend,
}

impl MergeIntent {
    pub fn from_header(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some(v) if v.eq_ignore_ascii_case("prepend") => Self::Prepend,
            _ => Self::Append,
        }
    }
}

impl ScrollMeta {
    /// Page-number pagination: `current` of `last` pages, 1-based.
    pub fn numbered(current: u64, last: u64) -> Self {
        let current = current.max(1);
        Self::cursor(
            Some(current),
            (current > 1).then(|| current - 1),
            (current < last).then(|| current + 1),
        )
    }

    /// Cursor (or any other) pagination; `None` means there is no such page.
    pub fn cursor(
        current: Option<impl Serialize>,
        previous: Option<impl Serialize>,
        next: Option<impl Serialize>,
    ) -> Self {
        Self {
            page_name: "page".to_string(),
            current_page: page_value(current),
            previous_page: page_value(previous),
            next_page: page_value(next),
            wrapper: "data".to_string(),
            match_on: Vec::new(),
        }
    }

    /// Query parameter carrying the page (default `page`); give each scroll prop on a page
    /// its own name.
    #[must_use]
    pub fn page_name(mut self, name: impl Into<String>) -> Self {
        self.page_name = name.into();
        self
    }

    /// Key of the items inside the prop value (default `data`).
    #[must_use]
    pub fn wrapper(mut self, wrapper: impl Into<String>) -> Self {
        self.wrapper = wrapper.into();
        self
    }

    /// De-duplicate merged items by this field (like [`crate::Prop::match_on`]).
    #[must_use]
    pub fn match_on(mut self, key: impl Into<String>) -> Self {
        self.match_on.push(key.into());
        self
    }

    pub(crate) fn to_json(&self, reset: bool) -> Value {
        serde_json::json!({
            "pageName": self.page_name,
            "previousPage": self.previous_page,
            "nextPage": self.next_page,
            "currentPage": self.current_page,
            "reset": reset,
        })
    }
}

fn page_value(page: Option<impl Serialize>) -> Value {
    page.map_or(Value::Null, |p| {
        serde_json::to_value(p).unwrap_or(Value::Null)
    })
}
