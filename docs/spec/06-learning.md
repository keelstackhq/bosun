## 6. Learning loop

A lesson is a failure that is not fixed in code yet. The preferred result of a lesson is a
mechanism (a gate, a hook, a check). Advice text is the last choice.

Capture:
- Automatic: bosun opens a lesson issue on a stop rule, a blocked task, a flake, a review ask that
  repeats, or a task over its budget.
- Manual: `bosun lesson add "<text>"`, or Slack intake with `--kind lesson`.
- `lesson add` needs `--evidence #N` (the PR or issue that proved it; exit 2 without). The core
  `lessons::add` enforces the rest for every caller (`lesson add`, `lesson triage --as advice
  --rule`, `hold`): a rule of at most 160 characters (exit 2 above, nothing written) and no
  duplicate. A rule equal to an existing one (lowercase, whitespace collapsed, trailing `.`
  ignored) records a hit on that entry instead, also at a cap: exit 0, `data.dup` and `data.id`
  are its id. The new scope globs and evidence refs are merged into the existing entry without
  repeats; a global entry (no scope) stays global, because adding a scope would narrow it.
  `--upstream` is unchanged.
- The compact lesson line (pack, claim, session start) drops control characters, collapses
  whitespace and cuts the rule to 160 characters, ending with `…`, so hand-written entries
  cannot swell the output.
- Duplicates: a lesson with the same signature adds a hit to the existing issue; it does not open
  a new one.
- Lesson issue: label `bosun:lesson`, title `lesson: <title>`, and the marker
  `<!-- bosun:lesson {"signature":"<hex>","kind":"<kind>"} -->` in the body. The signature is
  FNV-1a 64 of the kind and the normalised failure text (hex ids, times, numbers and temp paths
  replaced; first 20 lines). `bosun lesson capture --kind <k> [--issue N] --from -` opens the
  issue, or posts a `Seen again` comment with a `lesson-seen` record on the open one with the same
  signature; that record is the hit.

Triage (curator Routine, daily, Sonnet):
- blocking: sets process-red, so waves pause as on main-red. Process-red is an open issue labelled
  `bosun:process-red` (a blocking lesson carries it). While one is open, `claim` and `spawn` refuse
  work other than `mainfix` and `process-fix` with exit 4, and `wave plan` returns exit 4, except
  for a mainfix or process-fix epic.
- `bosun lesson triage <issue> --as blocking|mechanizable|advice` applies one of the labels
  `bosun:lesson/blocking`, `bosun:lesson/mechanizable`, `bosun:lesson/advice`; a re-triage removes
  the other two.
- mechanizable: opens a kind/process-fix task (`bosun:kind/process-fix` + `bosun:state/ready`),
  which runs before the features of the same area.
- advice only: adds an entry to `docs/lessons.toml`.
The lesson issue closes when its mechanism merges.

Store `docs/lessons.toml`. The first `lesson add` creates the file; a missing file is an empty
store (the file is absent until then, so there is nothing to add by hand):

```toml
[[lesson]]
id = "L-012"
rule = "Run cargo metadata once per gate run, not once per package."
why = "The test gate took 9 min on 40 packages."
scope = ["src/gates/**"]
evidence = ["#214", "#230"]
added = "2026-11-03"
last_hit = "2026-11-10"
hits = 3
```

- A rule is at most 160 characters. A pack or claim shows a lesson as one line
  `L-012 src/gates/** Run cargo metadata once per gate run.` (id, scopes joined by `,` or `*`
  when global, rule; no why or evidence). `review-pack` `lessons` is an array of these lines.
- Secrets: `lesson add` (the rule and `--why`), `handoff` (each line of the `--from` input), `msg`
  (the text) and `fail` (`--signature`) refuse text that looks like a secret (GitHub, `sk-`, AWS
  and Slack tokens, `Bearer` values, PEM headers, or a 32+ character high-entropy word) with exit 2.
  The error names the kind and the field, never the matched text; nothing is written. Fix: replace
  the secret with `<redacted>` and run again. Words are split on characters outside
  `A-Za-z0-9+/_=-`. A high-entropy word has 32+ characters, letters and digits, and Shannon
  entropy of at least 4.0 bits per character; hex words of 32+ characters are refused unless
  they are exactly 40 or 64 long (git shas), and JWTs (`eyJ` with three dot parts) are refused.
  Exempt: ids starting `session_` or `cse_`, `sha1-`/`sha256-`/`sha384-`/`sha512-` integrity
  hashes, snake or kebab phrases of 3+ lowercase words, and slash paths that are not mixed case.
  `sk-` needs a digit or at most 2 hyphens; `Bearer` needs a digit and a letter.
  Not covered yet (deferred): `lesson add --scope` and `--evidence`, `lesson capture`, `result`.
- Caps: at most 40 entries, at most 8 global (no scope).
- A context pack includes only the lessons whose scope matches the paths of the task, plus the
  global ones. AGENTS.md has no lessons.

Prune:
- `bosun lesson hit <id>` records a hit. `lesson add` and `lesson hit` change the local file `docs/lessons.toml` and need no GitHub; only `lesson hit --issue N`, which posts a `lesson-hit` record, does (and obeys the kill switch).
- At the cap, `bosun lesson add` refuses with exit 6. The curator must then mechanize a lesson,
  merge two lessons, or retire a lesson with no hits in 30 days.
- A lesson that gets a hit after its mechanism merged reopens as blocking. A lesson is mechanized
  when its issue is labelled `bosun:lesson/mechanizable` and closed as `completed`. When
  `lesson capture` finds no open issue with the signature but a mechanized one, it reopens that
  issue, makes it `bosun:lesson/blocking` (the other triage labels go), adds `bosun:process-red`, and comments "The mechanism did not
  hold: seen again. Reopened as blocking" with the `lesson-seen` record; the output has
  `reopened: true`. Issues closed as `not_planned` or `duplicate`, advice lessons and untriaged
  lessons are ignored: a new lesson opens.
- `bosun lesson add --upstream "<text>" [--why ...]` sends a lesson about bosun itself to
  keelstackhq/bosun as an issue (`bosun:lesson`, `bosun:source/lesson`; title cut to 120
  characters), not to the local store.
  It obeys the kill switch; data `{upstream: true, issue}`.
- Metrics: `bosun progress --lessons` (open, mechanized, hits per week). data `lessons: {open,
  blocking, mechanized, advice, hits_last_7_days, hits_by_week: [{week_start, hits}]}` (4 weeks,
  oldest first), counted from `lesson-seen` and `lesson-hit` records.
