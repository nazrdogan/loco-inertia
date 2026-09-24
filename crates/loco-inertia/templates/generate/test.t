{% set file_name = name | snake_case -%}
{% set module_name = file_name | pascal_case -%}
to: tests/{{ file_name }}_pages.rs
skip_exists: true
message: "Tests for `{{module_name}}` were added. Run `cargo test` (it also exports the props types)."
---
use {{pkg_name}}::app::App;
use loco_rs::testing::prelude::*;
{% for action in actions %}{% if action.page %}
#[tokio::test]
async fn renders_{{ action.name | snake_case }}() {
    request::<App, _, _>(|request, _ctx| async move {
        // A first visit: the HTML embeds the page object (no asset version involved).
        let res = request
            .get("/{{ file_name | plural }}{% if action.name != "index" %}/{{ action.name }}{% endif %}")
            .await;
        res.assert_status_ok();
        assert!(res
            .text()
            .contains(r#""component":"{{module_name}}/{{ action.name | pascal_case }}""#));
    })
    .await;
}
{% endif %}{% endfor -%}
