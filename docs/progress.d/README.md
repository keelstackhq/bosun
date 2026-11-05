# docs/progress.d

One file per PR, named after its branch with `/` as `-`: exactly one line, a Done line
(`- <date> · <task> · <what> · PR #<n>`). PRs never edit the Done history, so they never conflict
on it; Status and Blocked in `docs/PROGRESS.md` are still edited in place. `bash scripts/fold-fragments.sh`
appends the fragments to `## Done` of `docs/progress-done.md` (in file-name order; created when
missing) and deletes them. Agents never read `docs/progress-done.md`. This README is not a fragment.
