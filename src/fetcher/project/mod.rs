//! Project manifest fetchers — read project metadata (name, version, description, license)
//! from the nearest manifest file discovered by walking up from the process CWD.
//! Supports Cargo.toml, package.json, pyproject.toml, go.mod, and composer.json.
//! All fetchers are `Safety::Safe` (local file reads only).

use std::sync::Arc;

use super::Fetcher;

mod detect;
mod manifest;

pub use manifest::ProjectManifest;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(ProjectManifest)]
}
