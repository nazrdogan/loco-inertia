use std::collections::BTreeMap;

use serde::Serialize;

/// The Inertia page object sent either embedded in the root HTML or as the JSON body
/// of an Inertia response.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    /// The page component the client renders, e.g. `Users/Index`.
    pub component: String,
    /// The resolved props: a JSON object.
    pub props: serde_json::Value,
    /// Path and query of the request, e.g. `/users?page=2`.
    pub url: String,
    /// Asset version; an empty string when the server has no version configured.
    pub version: String,
    #[serde(skip_serializing_if = "is_false")]
    /// Make the client drop its history (and history encryption key).
    pub clear_history: bool,
    #[serde(skip_serializing_if = "is_false")]
    /// Make the client encrypt this page in `history.state`.
    pub encrypt_history: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    /// Prop paths the client appends to (or shallow-merges into) its current value.
    pub merge_props: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    /// Prop paths the client prepends to its current value.
    pub prepend_props: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    /// Prop paths the client merges into its current value recursively.
    pub deep_merge_props: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    /// `<prop>.<field>` paths: merged items with the same field replace each other.
    pub match_props_on: Vec<String>,
    /// Deferred prop paths by group; only present on full (non-partial) visits.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub deferred_props: BTreeMap<String, Vec<String>>,
    /// Pagination metadata of infinite scroll props, by prop path.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub scroll_props: BTreeMap<String, serde_json::Value>,
    /// Once props by key: `{ prop, expiresAt }`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub once_props: BTreeMap<String, serde_json::Value>,
    /// Top-level keys that came from shared props.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shared_props: Vec<String>,
    /// One-time data for this response (e.g. a toast after a redirect).
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub flash: serde_json::Map<String, serde_json::Value>,
}

impl Page {
    /// A page without merge, defer, once, flash or history metadata.
    pub fn new(
        component: impl Into<String>,
        props: serde_json::Value,
        url: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            component: component.into(),
            props,
            url: url.into(),
            version: version.into(),
            clear_history: false,
            encrypt_history: false,
            merge_props: Vec::new(),
            prepend_props: Vec::new(),
            deep_merge_props: Vec::new(),
            match_props_on: Vec::new(),
            deferred_props: BTreeMap::new(),
            scroll_props: BTreeMap::new(),
            once_props: BTreeMap::new(),
            shared_props: Vec::new(),
            flash: serde_json::Map::new(),
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Serialize a page for embedding inside `<script type="application/json">`.
///
/// `<` and `>` are emitted as `<` / `>` (like PHP's `JSON_HEX_TAG`) so a prop
/// containing `</script>` cannot terminate the script element. Both characters can only
/// appear inside JSON strings, where the escaped form is equivalent.
pub fn to_script_json(page: &Page) -> serde_json::Result<String> {
    let json = serde_json::to_string(page)?;
    Ok(json.replace('<', "\\u003c").replace('>', "\\u003e"))
}

/// The markup the Inertia client mounts on: the page data script followed by the root element.
pub fn mount_markup(app_id: &str, script_json: &str) -> String {
    format!(
        r#"<script data-page="{app_id}" type="application/json">{script_json}</script><div id="{app_id}"></div>"#
    )
}
