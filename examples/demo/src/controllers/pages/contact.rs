//! Live validation (Precognition), validation errors and flash messages across a redirect.

use axum::response::{IntoResponse, Response};
use loco_inertia::{Inertia, InertiaForm};
use loco_rs::Result;

use crate::views::{
    pages::{ContactForm, ContactProps},
    shared::AppFlash,
};

pub async fn show(inertia: Inertia) -> Result<Response> {
    // What the visitor typed is kept out of the browser history in plain text.
    Ok(inertia.encrypt_history(true).page(ContactProps {}).await)
}

/// `InertiaForm` answers the client's live validation requests and redirects back with the
/// errors when the submitted form is invalid; this only runs for valid input.
pub async fn submit(
    inertia: Inertia,
    InertiaForm(form): InertiaForm<ContactForm>,
) -> Result<Response> {
    let flash = AppFlash {
        message: Some(format!("Thanks {}, we got your message!", form.name)),
    };
    Ok(inertia
        .redirect("/contact")
        .with_flash("message", flash.message)
        .into_response())
}
