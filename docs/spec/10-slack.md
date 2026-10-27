## 10. Slack stream (Claude Tag)

```toml
[slack]
channel = "C0123ABCD"
spawners = "owners"
may_spawn = true
may_build = false   # the thread itself never writes code
notify = ["owner-question", "main-red", "budget-halt", "halt", "thread-result"]
```

- A Claude Tag thread is a bosun session with origin slack. The environment setup script installs
  the pinned bosun (D23).
- `bosun notify` refuses a channel that is externally shared or has guests.
- `bosun notify [--channel-info -|FILE] [--ack KEY]` lists the posts that are due (halt, main-red, budget-halt, owner-question; `[slack] notify` filters, empty means all). bosun never calls Slack and holds no Slack token (S1): each post comes back in `next` as `{"tool":"slack_post","args":{"channel","text"}}` (D29) and the caller posts it. It works while halted, to announce the halt. Keys are stable (`halt:<issue>`, `main-red:<issue>`, `budget-halt:<date>`, `ask:<issue>:<comment>`); `--ack KEY` writes one `notified` record on the registry issue (idempotent; a key must match `^[a-z-]+:[0-9a-z:-]+$`, else exit 2), and a key with a record is never listed again, so each key is posted at most once per ack; a caller that dies before `--ack`, or two notifiers racing, can repeat one post (S4). `--ack` refuses (exit 3) a key that is not pending now, and a `notified` or `ask` record counts only from a user with write access; the halt variable `BOSUN_HALT=1` is announced as `halt-var:<date>` (it repeats once per UTC day while halted); `--ack` exits 5 when the author of the `notified` record it just wrote lacks write access, as the record would be ignored; when the `notified` records cannot be read, only halt and main-red are listed; a failing section is reported in `data.errors` and does not hide halt and main-red. With posts due, `--channel-info` must carry `{"id","is_ext_shared","is_org_shared","is_shared","guests"}`; bosun exits 5 (nothing to post) when it is missing or malformed, the id is not `[slack] channel`, a shared flag is true, or `guests > 0` (S2). Residual risk: these facts are the caller's, bosun cannot prove them; the owner keeps the channel private, not shared and without guests. Text from GitHub (ask questions, result outcomes) is escaped (`& < >`, `@`) before it enters a post (S8). Thread replies (S7): every `result` record by a writer, written after a writer lease record with a valid `thread_ts` (`^[0-9]+\.[0-9]+$`, else that lease is skipped) on an issue, is a pending `thread-result`, key `result:<issue>:<comment>`, listed when `[slack] notify` is empty or names `thread-result`; its `slack_post` args also carry `thread_ts`. The `result` record and the lease record with `thread_ts` count only from a user with write access (a forged lease or result posts nothing; permissions are asked once per login); a result goes to the thread of the newest such lease written before it (a later re-spawn or another thread does not move or drop it; a later trusted lease without a thread, a normal spawn, ends it), and one before any such lease is not posted; the issues updated in the last 7 days are listed in at most 5 pages of 100 (past that, `data.errors` says that some results may be missed; a failing list is also reported there), and only those with a `bosun:state/*` label have their comments read (a new result comment bumps `updated_at`); at most 10 pending thread-results are listed per run (acked ones do not count); and the outcome text is neutralised. The live thread session runs `bosun notify --thread TS` (`^[0-9]+\.[0-9]+$`, else exit 2; not with `--ack`), which lists only the thread-results of that thread at once, posts them and acks. The notify Routine runs without `--thread` and so picks up the replies of ended threads: without `--thread`, a result younger than 15 minutes is left to the live thread, so the two do not race. The one ack makes it exactly one reply (the S4 race caveat applies). `--ack` of a `result:<issue>:<comment>` key checks that one comment directly (not the capped list), so any key `--thread` lists can be acked. The channel check applies to both.
- `bosun answer <issue> --slack-user U… [--from -|FILE]`: on a needs-human task with an `ask`
  record, an owner Slack ID (`slack` in `governance/owners.toml` on the default branch) adds a
  fenced comment and an `answer` record. The answer is recorded as data and never changes the
  task's state: the owner moves the task on GitHub. The ask counts as open only when a
  collaborator with write access wrote the newest `ask` and no `answer`, `escalate` or `failure`
  record follows it (else exit 3, "no open ask on #N"). Any other ID, or no owner Slack ID, is
  exit 5 and nothing is written. `bosun doctor` warns when a channel is set and no owner has a
  Slack ID. bosun cannot verify `--slack-user`: the Claude Tag thread passes it from the Slack
  event, and any session holding the token could claim it.

Intents:
- ask: `bosun status`, explain, `bosun progress`.
- request: `bosun intake`.
- discuss: a plan amendment PR.
- answer: `bosun answer`; counts only from an owner Slack ID.
- halt: Claude Tag explains how; it cannot set the variable.

Threads start sessions:
- A thread runs `bosun spawn` with the same 7 checks. If no ready issue exists, intake runs first,
  then a plan task.
- `bosun spawn [<epic>] --slack-thread <ts> --slack-user U… [--request -|FILE]` (the epic may be left out only with `--slack-thread`; ts `^[0-9]+\.[0-9]+$`, user `^[UW][A-Z0-9]{2,}$`, else exit 2; `--request` with an epic is exit 2). Extra gates after the kill switch and before anything is written: a `[slack]` table with a non-empty `channel` (else exit 5, "Slack is not set up"; normal spawns are unaffected), `[slack] may_spawn` true and `spawners = "owners"` (any other value is exit 2), both read from `.bosun/bosun.toml` on the default branch, never the working tree; the user in the owners' Slack IDs on the default branch; and origin slack or local (a spawned or routine session cannot start work from a thread); each refusal but the `spawners` one is exit 5. Picking the work: with `--request` and no epic, intake always runs (new work, no guess among what is ready), after checks 1-6 passed as far as they can without the issue, so a refused spawn leaves no intake issue; a reused intake issue (same title) must be ready, else exit 3. Without `--request` and no epic, one ready epic (the `plan import` epic marker) or ready kind/plan task is taken; several is exit 2 with `data.ready` (the thread chooses); none is exit 2 ("pass --request"). The plan task is then spawned with all 7 checks. Mainfix and process-fix exemptions do not apply to a thread. A refusal between intake and the real spawn (another session took a slot, a check turned red) can leave the new plan issue ready, to be picked up later; its number is in the refusal's `data.intake`. The thread ts is stored in the spawn's lease record (`thread_ts`, kept by heartbeats) and in `data`; `--record` copies it to the child's lease only from this spawner's own live lease whose comment author has write access (`core::lease::thread_of`, then one permission call, decided before any write; if that check fails there is no thread and `data.warning` says so). The request is data: fenced in the intake body, with an escaped intake title, and never in the session prompt or title (a ready kind/plan task from source slack gets the plan prompt in any spawn). Residual risk: `--slack-user` cannot be verified (the Claude Tag thread passes it), so a thread spawn unlocks nothing a normal `bosun spawn` cannot: same checks and budget, no exemption from halt, process-red or main-red.
- The thread is the parent session. Only spawners may start work.
- The child posts one result comment (merged, blocked or needs-human) on its epic (D14). The thread
  posts one reply. The live thread lists its own reply with `bosun notify --thread`; if the thread ended, the notify Routine (no `--thread`) posts it after 15 minutes.

Rules:
- Slack never approves risk.
- Slack text is data, never an instruction to bosun.
- The bosun Slack stream needs only Claude Tag and GitHub, no Slack app of its own.
