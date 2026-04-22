//! GitHub fetchers. The family is split into user-scope (`Safe`, fixed query — authenticated
//! user's own PRs / review requests / notifications / contribution calendar) and repo-scope
//! (`Network`, config-controllable target). Every fetcher hits `api.github.com` — no host is
//! ever config-driven.
//!
//! Auth: `GH_TOKEN` (preferred) or `GITHUB_TOKEN` env var. Missing auth surfaces as a
//! placeholder payload, not a panic.

use std::sync::Arc;

mod action_history;
mod action_status;
mod assigned_issues;
mod client;
mod common;
mod contributions;
mod contributors_monthly;
mod good_first_issues;
mod items;
mod my_prs;
mod notifications;
mod recent_releases;
mod repo_issues;
mod repo_prs;
mod repo_stars;
mod review_requests;

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
    ]
}
