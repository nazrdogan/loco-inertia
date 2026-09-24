use loco_inertia::Inertia;
use loco_rs::prelude::*;

use crate::views::pages::{AboutProps, HomeProps};

mod contact;
mod feed;
mod upload;

async fn home(inertia: Inertia) -> Result<Response> {
    Ok(inertia
        .page(HomeProps {
            greeting: "Hello from Loco!".to_string(),
            features: ["Rust controllers", "React 19 pages", "No API layer"]
                .map(String::from)
                .to_vec(),
        })
        .await)
}

async fn about(inertia: Inertia) -> Result<Response> {
    Ok(inertia
        .page(AboutProps {
            framework: "Loco.rs".to_string(),
            adapter_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .await)
}

pub fn routes() -> Routes {
    Routes::new()
        .add("/", get(home))
        .add("/about", get(about))
        .add("/feed", get(feed::show))
        .add("/contact", get(contact::show).post(contact::submit))
        .add("/upload", get(upload::show).post(upload::submit))
}
