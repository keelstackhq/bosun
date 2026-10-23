# Changelog

All notable changes to bosun. The format follows [Keep a Changelog](https://keepachangelog.com/);
bosun is unreleased, so every entry is under Unreleased.

## Unreleased

### Added

- `risk-check` accepts an owner's PR comment `/bosun approve <head sha>` (full 40-character SHA,
  unedited) as the approval of a high-risk PR, beside a GitHub review. The `risk` workflow also
  runs on such comments, from the default branch.
- Kill switch: an open issue labelled `bosun:halt` and opened by an owner halts bosun (exit 4).
  Create the label with `gh label create bosun:halt --repo <owner>/<repo> --color B60205`.

- `bosun verdict comment` posts a verdict as a PR comment, and the `verdict` workflow runs
  `bosun verdict apply <comment-id>` to check it and post the `review-verdict` status, so cloud
  sessions (which cannot write statuses) can deliver verdicts.

### Fixed

- The `verdict` workflow has `pull-requests: write`: with `issues: write` alone, its verdict record
  comment on the PR got 403 after the status was posted (first full run, #73).
- `bosun verdict apply` reads only the ```json block of a verdict comment: the agent proxy
  appends a footer to every comment, which made the second verdict comment on #73 invalid.
- `bosun verdict apply` checks the author's repository permission (write or admin) instead of
  `author_association`: the workflow token sees an org member as `CONTRIBUTOR`, so the first
  verdict comment (#73) was refused.

### Changed

- risk-check counts a `/bosun approve` comment only when GitHub says no app wrote it
  (`performed_via_github_app` present and `null`). Agent sessions comment as the owner's login
  through the Claude app, so before this an agent could approve its own PR.

- The `risk` workflow runs on `pull_request_target` instead of `pull_request`: the judging job
  comes from the base branch, never checks out PR-head code, passes event data through `env`
  only, and has least permissions. `tests/workflows.rs` asserts it.
- The repo variable `BOSUN_HALT` is now a second, optional kill-switch source: 404 (unset) and 403
  (not readable from this token) are no signal; any other read error still halts. Before, a 403
  halted every session behind the agent proxy.
- `bosun doctor` step 5 ("kill switch") checks the `bosun:halt` label instead of the variable.
