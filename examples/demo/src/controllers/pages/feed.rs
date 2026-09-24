//! Demonstrates infinite scroll, deferred and optional props.

use std::{convert::Infallible, time::Duration};

use loco_inertia::{Inertia, Prop, ScrollData, ScrollMeta};
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::views::pages::{FeedProps, Post, Stats};

const PER_PAGE: u32 = 10;
const TOTAL_POSTS: u32 = 100;

#[derive(Deserialize)]
pub struct FeedQuery {
    page: Option<u32>,
    /// Filter; the client resets the scroll prop when toggling it.
    even: Option<u8>,
}

/// One page of the (optionally filtered) posts and the number of pages.
fn posts_page(page: u32, even_only: bool) -> (Vec<Post>, u32) {
    let all: Vec<u32> = (1..=TOTAL_POSTS)
        .filter(|id| !even_only || id % 2 == 0)
        .collect();
    let last_page = all.len().div_ceil(PER_PAGE as usize) as u32;
    let page = page.clamp(1, last_page);
    let posts = all
        .chunks(PER_PAGE as usize)
        .nth(page as usize - 1)
        .unwrap_or_default()
        .iter()
        .map(|&id| Post {
            id,
            title: format!("Post #{id}"),
        })
        .collect();
    (posts, last_page)
}

pub async fn show(inertia: Inertia, Query(q): Query<FeedQuery>) -> Result<Response> {
    let even_only = q.even == Some(1);
    let page = q.page.unwrap_or(1).max(1);
    let (posts, last_page) = posts_page(page, even_only);
    let props = FeedProps {
        posts: ScrollData::new(posts),
        even_only,
        stats: None,
        categories: None,
    };
    Ok(inertia
        .page_with(props, |p| {
            // Pages are appended (or prepended when scrolling up) and de-duplicated by id.
            p.scroll(
                "posts",
                ScrollMeta::numbered(page.min(last_page).into(), last_page.into()).match_on("id"),
            )
            // Rendered first without it; the client fetches it right after.
            .with(
                "stats",
                Prop::defer(|| async {
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    Ok::<_, Infallible>(Stats {
                        total_posts: TOTAL_POSTS,
                        computed_in_ms: 600,
                    })
                }),
            )
            // Only sent when the client asks for it by name.
            .with(
                "categories",
                Prop::optional(|| async {
                    Ok::<_, Infallible>(["rust", "loco", "inertia"].map(String::from))
                }),
            )
        })
        .await)
}
