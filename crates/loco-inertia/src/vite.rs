//! Vite integration for the root template: script/style tags from the dev server or the
//! build manifest, and an asset version derived from the manifest.
//!
//! ```yaml
//! settings:
//!   inertia:
//!     vite:
//!       entry: src/main.tsx                           # default entry for `vite()`
//!       dev_server: http://localhost:5173            # development: serve from Vite
//!       manifest: frontend/dist/.vite/manifest.json  # production: `vite build --manifest`
//!       base: /static/                               # URL prefix of the built files
//!       react_refresh: true                          # dev: inject the React Refresh preamble
//! ```
//!
//! With `dev_server` set the dev server is used; otherwise tags come from `manifest`.
//! In the root template: `{{ vite() | safe }}` or `{{ vite(entry="src/admin.tsx") | safe }}`.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use loco_rs::{Error, Result};
use serde::{Deserialize, Serialize};

/// Default for `settings.inertia.vite.manifest`.
pub const DEFAULT_MANIFEST: &str = "frontend/dist/.vite/manifest.json";

/// `settings.inertia.vite`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ViteSettings {
    /// The entry `vite()` uses when the template names none, e.g. `src/main.tsx`.
    pub entry: Option<String>,
    /// Vite dev server origin; when set, tags point at it and the manifest is not read.
    pub dev_server: Option<String>,
    /// Build manifest; defaults to [`DEFAULT_MANIFEST`].
    pub manifest: Option<PathBuf>,
    /// URL prefix of the built files (Vite's `base`); defaults to `/`.
    pub base: Option<String>,
    #[serde(default)]
    /// Dev server: inject the `@vitejs/plugin-react` Refresh preamble.
    pub react_refresh: bool,
}

/// One chunk of the Vite build manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct Chunk {
    /// The built file, relative to `base`.
    pub file: String,
    #[serde(default)]
    /// Manifest keys of the chunks this one imports statically.
    pub imports: Vec<String>,
    #[serde(default)]
    /// CSS files of this chunk.
    pub css: Vec<String>,
}

#[derive(Debug, Clone)]
enum Mode {
    Dev {
        origin: String,
        react_refresh: bool,
    },
    Build {
        chunks: HashMap<String, Chunk>,
        base: String,
        version: String,
    },
}

/// Resolves entry points to HTML tags.
#[derive(Debug, Clone)]
pub struct Vite {
    mode: Mode,
    default_entry: Option<String>,
}

impl Vite {
    /// Tags pointing at a running Vite dev server.
    pub fn dev(origin: impl Into<String>, react_refresh: bool) -> Self {
        Self {
            mode: Mode::Dev {
                origin: origin.into().trim_end_matches('/').to_string(),
                react_refresh,
            },
            default_entry: None,
        }
    }

    /// Tags from a build manifest (`vite build` with `build.manifest: true`).
    pub fn from_manifest(path: impl AsRef<Path>, base: impl Into<String>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| {
            Error::Message(format!(
                "cannot read Vite manifest {path:?} ({e}); run `vite build` or set \
                 `settings.inertia.vite.dev_server`"
            ))
        })?;
        Self::from_manifest_bytes(&bytes, base)
    }

    /// Like [`Vite::from_manifest`], from the manifest's contents.
    pub fn from_manifest_bytes(bytes: &[u8], base: impl Into<String>) -> Result<Self> {
        let chunks: HashMap<String, Chunk> = serde_json::from_slice(bytes)
            .map_err(|e| Error::Message(format!("invalid Vite manifest: {e}")))?;
        let mut base = base.into();
        if !base.ends_with('/') {
            base.push('/');
        }
        Ok(Self {
            mode: Mode::Build {
                chunks,
                base,
                version: fnv1a_hex(bytes),
            },
            default_entry: None,
        })
    }

    /// From `settings.inertia.vite`: the dev server when `dev_server` is set, else the manifest.
    pub fn from_settings(settings: &ViteSettings) -> Result<Self> {
        let vite = match &settings.dev_server {
            Some(origin) => Self::dev(origin, settings.react_refresh),
            None => Self::from_manifest(
                settings
                    .manifest
                    .as_deref()
                    .unwrap_or(Path::new(DEFAULT_MANIFEST)),
                settings.base.as_deref().unwrap_or("/"),
            )?,
        };
        Ok(vite.with_default_entry(settings.entry.clone()))
    }

    #[must_use]
    /// The entry `vite()` uses when the template names none.
    pub fn with_default_entry(mut self, entry: Option<String>) -> Self {
        self.default_entry = entry;
        self
    }

    /// Asset version: a hash of the manifest, so every build changes it. `None` in dev.
    pub fn version(&self) -> Option<&str> {
        match &self.mode {
            Mode::Build { version, .. } => Some(version),
            Mode::Dev { .. } => None,
        }
    }

    /// The tags to put in `<head>` for `entry` (or the configured default entry).
    pub fn tags(&self, entry: Option<&str>) -> Result<String> {
        let entry = entry.or(self.default_entry.as_deref()).ok_or_else(|| {
            Error::Message("no Vite entry given and no default `entry` configured".into())
        })?;
        let mut html = String::new();
        match &self.mode {
            Mode::Dev {
                origin,
                react_refresh,
            } => {
                if *react_refresh {
                    let _ = write!(
                        html,
                        r#"<script type="module">import RefreshRuntime from "{origin}/@react-refresh";RefreshRuntime.injectIntoGlobalHook(window);window.$RefreshReg$ = () => {{}};window.$RefreshSig$ = () => (type) => type;window.__vite_plugin_react_preamble_installed__ = true;</script>"#,
                        origin = attr(origin)
                    );
                }
                let _ = write!(
                    html,
                    r#"<script type="module" src="{o}/@vite/client"></script><script type="module" src="{o}/{e}"></script>"#,
                    o = attr(origin),
                    e = attr(entry.trim_start_matches('/'))
                );
            }
            Mode::Build { chunks, base, .. } => {
                let chunk = chunks.get(entry).ok_or_else(|| {
                    Error::Message(format!("Vite entry `{entry}` is not in the manifest"))
                })?;
                let mut css = Vec::new();
                let mut preload = Vec::new();
                let mut seen = HashSet::new();
                collect(chunks, chunk, &mut seen, &mut css, &mut preload);
                for file in dedup(css) {
                    let _ = write!(
                        html,
                        r#"<link rel="stylesheet" href="{}">"#,
                        attr(&format!("{base}{file}"))
                    );
                }
                for file in dedup(preload) {
                    let _ = write!(
                        html,
                        r#"<link rel="modulepreload" href="{}">"#,
                        attr(&format!("{base}{file}"))
                    );
                }
                let _ = write!(
                    html,
                    r#"<script type="module" src="{}"></script>"#,
                    attr(&format!("{base}{}", chunk.file))
                );
            }
        }
        Ok(html)
    }

    /// Register `vite(entry=?)` on a Tera instance.
    pub fn register(self: &Arc<Self>, tera: &mut tera::Tera) {
        let vite = Arc::clone(self);
        tera.register_function(
            "vite",
            move |args: &std::collections::HashMap<String, tera::Value>| {
                let entry = match args.get("entry") {
                    Some(tera::Value::String(s)) => Some(s.as_str()),
                    Some(other) => {
                        return Err(format!("vite(entry=…) must be a string, got {other}").into())
                    }
                    None => None,
                };
                vite.tags(entry)
                    .map(tera::Value::String)
                    .map_err(|e| e.to_string().into())
            },
        );
    }
}

/// CSS of the chunk and everything it statically imports, plus the imported chunk files
/// for `modulepreload` (dynamic imports stay lazy).
fn collect<'a>(
    chunks: &'a HashMap<String, Chunk>,
    chunk: &'a Chunk,
    seen: &mut HashSet<&'a str>,
    css: &mut Vec<&'a str>,
    preload: &mut Vec<&'a str>,
) {
    css.extend(chunk.css.iter().map(String::as_str));
    for key in &chunk.imports {
        if !seen.insert(key) {
            continue;
        }
        if let Some(imported) = chunks.get(key) {
            preload.push(&imported.file);
            collect(chunks, imported, seen, css, preload);
        }
    }
}

fn dedup(items: Vec<&str>) -> Vec<&str> {
    let mut seen = BTreeMap::new();
    items
        .into_iter()
        .filter(|i| seen.insert(*i, ()).is_none())
        .collect()
}

/// Escape for a double-quoted HTML attribute.
fn attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 64-bit FNV-1a as hex: stable across builds and platforms, unlike `DefaultHasher`.
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}
