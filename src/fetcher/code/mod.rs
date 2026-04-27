//! Code-internal metric fetchers — grep / parse the source tree of the discovered git repo.
//! All `Safety::Safe` (local file reads only).
//!
//! Distinct from the future `project_*` family in #62, which is for project-level operational
//! state (test coverage, bundle size, dependency alerts, license check). `code_*` is "what's
//! inside the source files".

use std::sync::Arc;

use super::Fetcher;

mod language_logo;
mod languages;
mod loc;
mod logo_assets;
mod scan;
mod todos;

pub use language_logo::CodeLanguageLogo;
pub use loc::CodeLoc;
pub use todos::CodeTodos;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(CodeTodos),
        Arc::new(CodeLoc),
        Arc::new(CodeLanguageLogo),
    ]
}
