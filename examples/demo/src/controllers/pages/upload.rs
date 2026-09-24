//! File uploads: a multipart form with a single and a multiple file input.

use axum::response::{IntoResponse, Response};
use loco_inertia::{Inertia, InertiaForm, UploadedFile};
use loco_rs::Result;
use serde::Deserialize;
use validator::{Validate, ValidationError};

use crate::views::pages::UploadProps;

const MAX_BYTES: usize = 1024 * 1024;

/// What the page submits. Not exported to TypeScript: the frontend holds `File`s.
#[derive(Debug, Deserialize, Validate)]
pub struct UploadForm {
    #[validate(length(min = 2, message = "Please tell us your name."))]
    name: String,
    #[validate(custom(function = "image_up_to_1mb"))]
    avatar: Option<UploadedFile>,
    #[serde(default)]
    #[validate(custom(function = "images_up_to_1mb"))]
    photos: Vec<UploadedFile>,
}

fn image_up_to_1mb(file: &UploadedFile) -> std::result::Result<(), ValidationError> {
    let is_image = file
        .content_type
        .as_deref()
        .is_some_and(|t| t.starts_with("image/"));
    if !is_image {
        return Err(ValidationError::new("image").with_message("Only images are allowed.".into()));
    }
    if file.bytes.len() > MAX_BYTES {
        return Err(
            ValidationError::new("size").with_message("Images must be at most 1 MB.".into())
        );
    }
    Ok(())
}

fn images_up_to_1mb(files: &[UploadedFile]) -> std::result::Result<(), ValidationError> {
    files.iter().try_for_each(image_up_to_1mb)
}

fn describe(file: &UploadedFile) -> String {
    format!(
        "{} ({}, {} bytes)",
        file.file_name.as_deref().unwrap_or("unnamed"),
        file.content_type.as_deref().unwrap_or("unknown type"),
        file.bytes.len()
    )
}

pub async fn show(inertia: Inertia) -> Result<Response> {
    Ok(inertia.page(UploadProps {}).await)
}

pub async fn submit(
    inertia: Inertia,
    InertiaForm(form): InertiaForm<UploadForm>,
) -> Result<Response> {
    let avatar = form
        .avatar
        .as_ref()
        .map_or("no avatar".to_string(), describe);
    let photos: Vec<String> = form.photos.iter().map(describe).collect();
    let message = format!(
        "Thanks {}: {avatar}; {} photo(s){}",
        form.name,
        photos.len(),
        if photos.is_empty() {
            String::new()
        } else {
            format!(": {}", photos.join(", "))
        }
    );
    Ok(inertia
        .redirect("/upload")
        .with_flash("message", message)
        .into_response())
}
