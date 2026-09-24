use loco_inertia::{InertiaPage, ScrollData};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct HomeProps {
    pub greeting: String,
    pub features: Vec<String>,
}

impl InertiaPage for HomeProps {
    const COMPONENT: &'static str = "Home";
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct AboutProps {
    pub framework: String,
    pub adapter_version: String,
}

impl InertiaPage for AboutProps {
    const COMPONENT: &'static str = "About";
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Post {
    pub id: u32,
    pub title: String,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Stats {
    pub total_posts: u32,
    pub computed_in_ms: u32,
}

/// `posts` is an infinite scroll prop (`<InfiniteScroll data="posts">`). `stats` (deferred)
/// and `categories` (optional) are added as lazy props by the controller; they are declared
/// here so the frontend type has them.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct FeedProps {
    pub posts: ScrollData<Post>,
    pub even_only: bool,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<Stats>,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
}

impl InertiaPage for FeedProps {
    const COMPONENT: &'static str = "Feed";
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ContactProps {}

impl InertiaPage for ContactProps {
    const COMPONENT: &'static str = "Contact";
}

/// The contact form, also the type of the frontend `useForm` data.
#[derive(Debug, Default, Deserialize, Serialize, TS, validator::Validate)]
#[ts(export)]
pub struct ContactForm {
    #[validate(length(min = 2, message = "Please tell us your name."))]
    pub name: String,
    #[validate(email(message = "That does not look like an email address."))]
    pub email: String,
    #[validate(length(min = 10, message = "The message needs at least 10 characters."))]
    pub message: String,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct UploadProps {}

impl InertiaPage for UploadProps {
    const COMPONENT: &'static str = "Upload";
}
