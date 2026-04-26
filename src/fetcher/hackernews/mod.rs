//! `hackernews_*` fetcher family. Splits into:
//!
//! - `top` — story listings (top / new / best / ask / show / job)
//! - `user` — profile rollup (login / karma / about / created / submission count)
//! - `user_submissions` — recent stories submitted by a user
//! - `user_comments` — recent comments by a user
//!
//! Shared HTTP client and base URLs live in `client`.

pub mod client;
pub mod top;
pub mod user;
pub mod user_comments;
pub mod user_submissions;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(top::HackernewsTopFetcher),
        Arc::new(user::HackernewsUserFetcher),
        Arc::new(user_submissions::HackernewsUserSubmissionsFetcher),
        Arc::new(user_comments::HackernewsUserCommentsFetcher),
    ]
}
