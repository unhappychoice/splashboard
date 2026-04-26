//! `reddit_*` fetcher family (OAuth-free subset). All `Safety::Safe` — host is fixed at
//! `www.reddit.com` and config-supplied subreddit / user names are normalized to a strict
//! ASCII alphanum subset, so traffic can never be redirected off-host.

mod client;
mod common;
mod subreddit_posts;
mod user_comments;
mod user_posts;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(subreddit_posts::RedditSubredditPostsFetcher),
        Arc::new(user_posts::RedditUserPostsFetcher),
        Arc::new(user_comments::RedditUserCommentsFetcher),
    ]
}
