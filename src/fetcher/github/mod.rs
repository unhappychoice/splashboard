//! GitHub fetchers. All classify as `Safety::Safe` because every request targets
//! `api.github.com` — the host is never config-driven, so the auth token can't be redirected
//! to an attacker-controlled origin. Config-provided `repo` / `login` only change which
//! resource within github.com is queried, and the response only ever renders on the user's
//! own screen with data their own token was already authorised for.
//!
//! Auth: `GH_TOKEN` (preferred) or `GITHUB_TOKEN` env var. Missing auth surfaces as a
//! placeholder payload, not a panic.

use std::sync::Arc;

mod action_history;
mod action_status;
mod assigned_issues;
mod avatar;
pub(crate) mod client;
pub(crate) mod common;
mod contributions;
mod contributors_monthly;
mod good_first_issues;
mod items;
mod languages;
mod my_prs;
mod notifications;
mod recent_releases;
mod repo_issues;
mod repo_prs;
mod repo_stars;
mod review_requests;
mod user;

use super::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(my_prs::GithubMyPrs),
        Arc::new(review_requests::GithubReviewRequests),
        Arc::new(assigned_issues::GithubAssignedIssues),
        Arc::new(notifications::GithubNotifications),
        Arc::new(repo_prs::GithubRepoPrs),
        Arc::new(repo_issues::GithubRepoIssues),
        Arc::new(repo_stars::GithubRepoStars),
        Arc::new(recent_releases::GithubRecentReleases),
        Arc::new(action_status::GithubActionStatus),
        Arc::new(action_history::GithubActionHistory),
        Arc::new(good_first_issues::GithubGoodFirstIssues),
        Arc::new(contributors_monthly::GithubContributorsMonthly),
        Arc::new(contributions::GithubContributions),
        Arc::new(languages::GithubLanguages),
        Arc::new(avatar::GithubAvatar),
        Arc::new(user::GithubUser),
    ]
}
