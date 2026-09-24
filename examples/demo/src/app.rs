use async_trait::async_trait;
use axum::{extract::Request, middleware::Next, response::Response, Router as AxumRouter};
use loco_inertia::{InertiaLayer, Prop, SharedProps};
use loco_rs::{
    app::{AppContext, Hooks, Initializer},
    bgworker::Queue,
    boot::{create_app, BootResult, StartMode},
    config::Config,
    controller::{
        middleware::{self, MiddlewareLayer},
        AppRoutes,
    },
    environment::Environment,
    task::Tasks,
    Result,
};

use crate::{controllers, views::shared::AppShared};

pub struct App;

#[async_trait]
impl Hooks for App {
    fn app_name() -> &'static str {
        env!("CARGO_CRATE_NAME")
    }

    async fn boot(
        mode: StartMode,
        environment: &Environment,
        config: Config,
    ) -> Result<BootResult> {
        create_app::<Self>(mode, environment, config).await
    }

    async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
        Ok(vec![])
    }

    fn middlewares(ctx: &AppContext) -> Vec<Box<dyn MiddlewareLayer>> {
        // Innermost first: Inertia runs inside Loco's request-id, logging and error layers.
        let mut stack: Vec<Box<dyn MiddlewareLayer>> = vec![Box::new(InertiaLayer::new(ctx))];
        stack.extend(middleware::default_middleware_stack(ctx));
        stack
    }

    async fn after_routes(router: AxumRouter, _ctx: &AppContext) -> Result<AxumRouter> {
        Ok(router.layer(axum::middleware::from_fn(share_props)))
    }

    fn routes(_ctx: &AppContext) -> AppRoutes {
        AppRoutes::with_default_routes().add_route(controllers::pages::routes())
    }

    async fn connect_workers(_ctx: &AppContext, _queue: &Queue) -> Result<()> {
        Ok(())
    }

    fn register_tasks(tasks: &mut Tasks) {
        tasks.register(loco_inertia::InertiaGenerate::new(Self::app_name()));
    }
}

/// Props every page receives (the `usePage().props` shared across the app).
async fn share_props(mut req: Request, next: Next) -> Response {
    let shared = AppShared {
        app_name: "Loco + Inertia".to_string(),
        loaded_at: None,
    };
    let props = SharedProps::of(req.extensions_mut());
    if let Err(e) = props.extend(shared) {
        tracing::error!("cannot share props: {e}");
    }
    props.insert(
        "loaded_at",
        Prop::lazy(|| async { Ok::<_, std::convert::Infallible>(clock_time()) }).once(),
    );
    next.run(req).await
}

/// `HH:MM:SS` (UTC) of now.
fn clock_time() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!(
        "{:02}:{:02}:{:02} UTC",
        secs / 3600 % 24,
        secs / 60 % 60,
        secs % 60
    )
}
