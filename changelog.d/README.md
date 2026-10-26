# changelog.d

One file per PR, named after its branch with `/` as `-` (`claude/foo` -> `claude-foo.md`):
exactly one line `- <entry>`, worded like a CHANGELOG.md bullet. PRs never edit CHANGELOG.md, so
they never conflict on it. Before a release, a docs PR to main runs `scripts/fold-fragments.sh`,
which appends the fragments under `## Unreleased` (in file-name order) and deletes them. Any
other file here (a subdirectory, a `.txt`) makes the script fail. This README is not a fragment.
