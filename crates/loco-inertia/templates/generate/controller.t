{% set file_name = name | snake_case -%}
{% set module_name = file_name | pascal_case -%}
to: src/controllers/{{ file_name }}.rs
skip_exists: true
message: "Inertia controller `{{module_name}}` was added successfully."
injections:
- into: src/controllers/mod.rs
  append: true
  content: "pub mod {{ file_name }};"
- into: src/app.rs
  after: "AppRoutes::"
  content: "            .add_route(controllers::{{ file_name }}::routes())"
---
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unused_async)]
use loco_inertia::{Inertia, InertiaPage};
use loco_rs::prelude::*;
use serde::Serialize;
use ts_rs::TS;

{% for action in actions -%}
{% set page_name = action.name | pascal_case -%}
{% if action.page -%}
/// Props of the `{{module_name}}/{{page_name}}` page
/// (`frontend/src/Pages/{{module_name}}/{{page_name}}.tsx`). `cargo test` exports the
/// TypeScript type.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct {{module_name}}{{page_name}}Props {
    pub title: String,
}

impl InertiaPage for {{module_name}}{{page_name}}Props {
    const COMPONENT: &'static str = "{{module_name}}/{{page_name}}";
}

#[debug_handler]
pub async fn {{action.name}}(inertia: Inertia) -> Result<Response> {
    Ok(inertia
        .page({{module_name}}{{page_name}}Props {
            title: "{{module_name}} {{action.name}}".to_string(),
        })
        .await)
}
{% else -%}
/// `{{action.verb | upper}}`: do the work, then send the user back with a flash message
/// (use `.with_errors(..)` for validation failures).
#[debug_handler]
pub async fn {{action.name}}(inertia: Inertia) -> Result<Response> {
    Ok(inertia
        .back()
        .with_flash("message", "{{module_name}} {{action.name}}: done")
        .into_response())
}
{% endif %}
{% endfor -%}
pub fn routes() -> Routes {
    Routes::new()
        .prefix("{{file_name | plural}}/")
        {%- for action in actions %}
        .add("{% if action.name == "index" %}/{% else %}{{action.name}}{% endif %}", {{action.verb}}({{action.name}}))
        {%- endfor %}
}
