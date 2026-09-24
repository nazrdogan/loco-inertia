//! The SSR circuit breaker: a failing renderer is skipped for a cooldown.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use axum::{body::Body, http::Request, routing::get, Extension, Router};
use http_body_util::BodyExt;
use inertia_core::{Inertia, InertiaConfig, Page, RootView, SsrFuture, SsrRenderer, SsrResponse};
use tower::ServiceExt;

/// A renderer that counts calls and fails until told otherwise.
#[derive(Clone, Default)]
struct Flaky {
    calls: Arc<AtomicUsize>,
    healthy: Arc<AtomicBool>,
    delay: Duration,
}

impl SsrRenderer for Flaky {
    fn render<'a>(&'a self, _page: &'a Page) -> SsrFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            if self.healthy.load(Ordering::SeqCst) {
                Ok(SsrResponse {
                    head: vec![],
                    body: "<div>ssr</div>".into(),
                })
            } else {
                Err("boom".into())
            }
        })
    }
}

fn app(flaky: Flaky, threshold: u32, cooldown: Duration) -> Router {
    let config = InertiaConfig::new(|v: &RootView<'_>| Ok(v.body.to_string()))
        .with_ssr_breaker(flaky, threshold, cooldown);
    Router::new()
        .route(
            "/",
            get(|i: Inertia| async move { i.render("Home", ()).await }),
        )
        .layer(Extension(config))
}

async fn visit(app: &Router) -> String {
    let res = app
        .clone()
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
}

#[tokio::test]
async fn opens_after_consecutive_failures_and_probes_after_the_cooldown() {
    let flaky = Flaky::default();
    let app = app(flaky.clone(), 3, Duration::from_millis(200));

    for _ in 0..6 {
        // Every visit still renders (client-side).
        assert!(visit(&app).await.contains("data-page"));
    }
    assert_eq!(
        flaky.calls.load(Ordering::SeqCst),
        3,
        "calls stop once the circuit opens"
    );

    tokio::time::sleep(Duration::from_millis(250)).await;
    visit(&app).await;
    assert_eq!(
        flaky.calls.load(Ordering::SeqCst),
        4,
        "one probe after the cooldown"
    );
    visit(&app).await;
    assert_eq!(
        flaky.calls.load(Ordering::SeqCst),
        4,
        "a failed probe reopens the circuit"
    );
}

#[tokio::test]
async fn a_successful_probe_closes_the_circuit() {
    let flaky = Flaky::default();
    let app = app(flaky.clone(), 2, Duration::from_millis(150));
    visit(&app).await;
    visit(&app).await;
    visit(&app).await;
    assert_eq!(flaky.calls.load(Ordering::SeqCst), 2);

    flaky.healthy.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(visit(&app).await, "<div>ssr</div>");
    assert_eq!(visit(&app).await, "<div>ssr</div>");
    assert_eq!(flaky.calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn occasional_failures_do_not_open_it() {
    let flaky = Flaky::default();
    let app = app(flaky.clone(), 3, Duration::from_secs(60));
    for _ in 0..3 {
        flaky.healthy.store(false, Ordering::SeqCst);
        visit(&app).await;
        visit(&app).await;
        flaky.healthy.store(true, Ordering::SeqCst);
        assert_eq!(
            visit(&app).await,
            "<div>ssr</div>",
            "a success resets the count"
        );
    }
    assert_eq!(flaky.calls.load(Ordering::SeqCst), 9);
}

#[tokio::test]
async fn only_one_request_probes_a_half_open_circuit() {
    let flaky = Flaky {
        delay: Duration::from_millis(100),
        ..Default::default()
    };
    let app = app(flaky.clone(), 1, Duration::from_millis(100));
    visit(&app).await;
    assert_eq!(flaky.calls.load(Ordering::SeqCst), 1);

    tokio::time::sleep(Duration::from_millis(150)).await;
    let visits: Vec<_> = (0..5)
        .map(|_| {
            let app = app.clone();
            tokio::spawn(async move { visit(&app).await })
        })
        .collect();
    for v in visits {
        v.await.unwrap();
    }
    assert_eq!(
        flaky.calls.load(Ordering::SeqCst),
        2,
        "one probe among concurrent visits"
    );
}

#[tokio::test]
async fn a_hung_renderer_stops_costing_latency() {
    // Each call takes 300 ms (like a timeout); after 2 the circuit opens.
    let flaky = Flaky {
        delay: Duration::from_millis(300),
        ..Default::default()
    };
    let app = app(flaky.clone(), 2, Duration::from_secs(60));
    visit(&app).await;
    visit(&app).await;
    let started = Instant::now();
    visit(&app).await;
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_cancelled_probe_does_not_block_later_probes() {
    let flaky = Flaky {
        delay: Duration::from_millis(200),
        ..Default::default()
    };
    let app = app(flaky.clone(), 1, Duration::from_millis(50));
    visit(&app).await;
    assert_eq!(flaky.calls.load(Ordering::SeqCst), 1);

    // The probe's client disconnects before the renderer answers.
    tokio::time::sleep(Duration::from_millis(80)).await;
    let _ = tokio::time::timeout(Duration::from_millis(20), visit(&app)).await;
    assert_eq!(flaky.calls.load(Ordering::SeqCst), 2);

    flaky.healthy.store(true, Ordering::SeqCst);
    assert_eq!(
        visit(&app).await,
        "<div>ssr</div>",
        "the next request probes"
    );
    assert_eq!(flaky.calls.load(Ordering::SeqCst), 3);
}
