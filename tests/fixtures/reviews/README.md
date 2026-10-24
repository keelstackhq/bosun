# Recorded reviews

One JSON file per PR: `{"pr": N, "head_sha": "...", "a": <Verdict>, "b": <Verdict>}`.

- `a`: the verdict of review mode a (a Claude session, `bosun verdict post`), for that head.
- `b`: the verdict of review mode b (`bosun review-ci <pr> --dry-run`, `data.verdict`), same head.
- Both are `core::verdict::Verdict` (unknown fields are refused).
- **Agree** (`core::agreement::agrees`): both are approved, or both are not approved, under
  `effective` (spec §7: confidence >= 0.85, no blocker) and no `escalate_risk`.
  `approve` at 0.84 and `request-changes` agree.

`since_sha` is an optional field: null or absent in real recordings, which are full
reviews (`--dry-run` always is one). A dry run's cost is not recorded as spend, so it does
not count against `daily_usd`.

Real recordings go directly in this directory, one file per PR, at the head both modes saw.
`synthetic/` holds made-up files that test the harness only; they never count.

UC-M8-3 ("on 20 recorded PRs, mode b agrees with mode a on at least 18") stays OPEN until 20
real recorded runs exist here. A real `uc_m8_3` test lands with them. Until then
`recorded_reviews_parse_and_pass_once_twenty_exist` only checks that recordings parse.
