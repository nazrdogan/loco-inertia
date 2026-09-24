use serde::Serialize;
use ts_rs::TS;

/// Props every page receives; see `frontend/src/inertia.d.ts`.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct AppShared {
    pub app_name: String,
    /// When the shared data was first loaded: a once prop, so it stays the same while the
    /// visitor navigates (the server skips it once the client has it).
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loaded_at: Option<String>,
}

/// `page.flash` after a redirect.
#[derive(Debug, Default, Serialize, TS)]
#[ts(export)]
pub struct AppFlash {
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
