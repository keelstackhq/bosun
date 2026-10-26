# docs/progress.d

One file per PR, named after its branch with `/` as `-`: exactly one line, the Done line of `docs/PROGRESS.md`
(`- <date> · <task> · <what> · PR #<n>`). PRs never append to PROGRESS.md's Done list, so they
never conflict on it; Status and Blocked are still edited in place. `scripts/fold-fragments.sh`
appends the fragments to `## Done` (in file-name order) and deletes them. This README is not a
fragment.
