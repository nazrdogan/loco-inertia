//! Page props and shared data, typed on both sides: `cargo test` exports each
//! `#[ts(export)]` type to `frontend/src/types/` (see `.cargo/config.toml`).

pub mod pages;
pub mod shared;
