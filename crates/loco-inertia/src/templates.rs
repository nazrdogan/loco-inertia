//! `cargo loco task inertia_generate name:posts actions:index,show,create` generates an
//! Inertia controller, one `.tsx` page per GET action and request tests.
//!
//! Loco 1.x's own `generate controller` only emits JSON API controllers from a single
//! template, so this task renders its own templates with the same engine (`rrgen`), including
//! the `mod.rs` / `app.rs` route injections. Like Loco, `create` is wired as POST, `update` as
//! PUT and `delete`/`destroy` as DELETE; those handlers redirect back with a flash message
//! instead of rendering a page.

use std::path::Path;

use async_trait::async_trait;
use loco_rs::{
    app::AppContext,
    task::{Task, TaskInfo, Vars},
    Error, Result,
};
use rrgen::{GenResult, RRgen};
use serde_json::json;

const CONTROLLER: &str = include_str!("../templates/generate/controller.t");
const PAGE: &str = include_str!("../templates/generate/page.t");
const TEST: &str = include_str!("../templates/generate/test.t");

/// The HTTP method for an action, following Loco's generator.
pub fn verb_for(action: &str) -> &'static str {
    match action {
        "create" => "post",
        "update" => "put",
        "delete" | "destroy" => "delete",
        _ => "get",
    }
}

/// Generate into `root` (the app directory). `pkg_name` is the app's crate name, used by
/// the generated tests. Returns the generator messages.
pub fn generate(
    root: impl AsRef<Path>,
    pkg_name: &str,
    name: &str,
    actions: &[String],
) -> Result<Vec<String>> {
    if name.is_empty() {
        return Err(Error::Message("`name` is required, e.g. name:posts".into()));
    }
    let default_actions = ["index".to_string()];
    let actions = if actions.is_empty() {
        &default_actions[..]
    } else {
        actions
    };
    let actions: Vec<_> = actions
        .iter()
        .map(|a| {
            let verb = verb_for(a);
            json!({ "name": a, "verb": verb, "page": verb == "get" })
        })
        .collect();
    let vars = json!({ "name": name, "actions": actions, "pkg_name": pkg_name });

    let rrgen = RRgen::with_working_dir(root);
    let mut messages = Vec::new();
    let mut run = |template: &str, vars: &serde_json::Value| -> Result<()> {
        match rrgen
            .generate(template, vars)
            .map_err(|e| Error::Message(e.to_string()))?
        {
            GenResult::Generated { message: Some(m) } => messages.push(m),
            GenResult::Generated { message: None } | GenResult::Skipped => {}
        }
        Ok(())
    };
    run(CONTROLLER, &vars)?;
    for action in actions.iter().filter(|a| a["page"] == true) {
        run(PAGE, &json!({ "name": name, "action": action }))?;
    }
    run(TEST, &vars)?;
    Ok(messages)
}

/// `cargo loco task inertia_generate name:<controller> actions:<a,b,c>`.
///
/// Register with the app's crate name: `tasks.register(InertiaGenerate::new(Self::app_name()))`.
pub struct InertiaGenerate {
    pkg_name: String,
}

impl InertiaGenerate {
    /// The task for the app crate `pkg_name` (`Self::app_name()` in `Hooks`).
    pub fn new(pkg_name: impl Into<String>) -> Self {
        Self {
            pkg_name: pkg_name.into(),
        }
    }
}

#[async_trait]
impl Task for InertiaGenerate {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "inertia_generate".to_string(),
            detail: "Generate an Inertia controller with typed pages: name:posts actions:index,show,create"
                .to_string(),
        }
    }

    async fn run(&self, _ctx: &AppContext, vars: &Vars) -> Result<()> {
        let name = vars.cli.get("name").map(String::as_str).unwrap_or_default();
        let actions: Vec<String> = vars
            .cli
            .get("actions")
            .map(|a| {
                a.split(',')
                    .map(str::trim)
                    .filter(|a| !a.is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();
        for message in generate(".", &self.pkg_name, name, &actions)? {
            println!("* {message}");
        }
        println!("Run `cargo test` to export the props types to the frontend.");
        Ok(())
    }
}
