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
- `bosun notify [--channel-info -|FILE] [--ack KEY]` lists the posts that are due (halt, main-red, budget-halt, owner-question; `[slack] notify` filters, empty means all). Slack is optional: without a `[slack]` table in `.bosun/bosun.toml` it exits 0 with `data.skipped` true, a `data.reason` and nothing pending; a `[slack]` table with an empty `channel` is exit 1. bosun never calls Slack and holds no Slack token (S1): each post comes back in `next` as `{"tool":"slack_post","args":{"channel","text"}}` (D29) and the caller posts it. It works while halted, to announce the halt. Keys are stable (`halt:<issue>`, `main-red:<issue>`, `budget-halt:<date>`, `ask:<issue>:<comment>`); `--ack KEY` writes one `notified` record on the registry issue (idempotent; a key must match `^[a-z-]+:[0-9a-z:-]+$`, else exit 2), and a key with a record is never listed again, so each key is posted at most once per ack; a caller that dies before `--ack`, or two notifiers racing, can repeat one post (S4). `--ack` refuses (exit 3) a key that is not pending now, and a `notified` or `ask` record counts only from a user with write access; the halt variable `BOSUN_HALT=1` is announced as `halt-var:<date>` (it repeats once per UTC day while halted); `--ack` exits 5 when the author of the `notified` record it just wrote lacks write access, as the record would be ignored; when the `notified` records cannot be read, only halt and main-red are listed; a failing section is reported in `data.errors` and does not hide halt and main-red. With posts due, `--channel-info` must carry `{"id","is_ext_shared","is_org_shared","is_shared","guests"}`; bosun exits 5 (nothing to post) when it is missing or malformed, the id is not `[slack] channel`, a shared flag is true, or `guests > 0` (S2). Residual risk: these facts are the caller's, bosun cannot prove them; the owner keeps the channel private, not shared and without guests. Text from GitHub (ask questions, result outcomes) is escaped (`& < >`, `@`) before it enters a post (S8). Thread replies (S7): every `result` record by a writer, written after a writer lease record with a valid `thread_ts` (`^[0-9]+\.[0-9]+$`, else that lease is skipped) on an issue, is a pending `thread-result`, key `result:<issue>:<comment>`, listed when `[slack] notify` is empty or names `thread-result`; its `slack_post` args also carry `thread_ts`. The `result` record and the lease record with `thread_ts` count only from a user with write access (a forged lease or result posts nothing; permissions are asked once per login); a result goes to the thread of the newest such lease written before it (a later re-spawn or another thread does not move or drop it; a later trusted lease without a thread, a normal spawn, ends it), and one before any such lease is not posted; the issues updated in the last 7 days are listed in at most 5 pages of 100 (past that, `data.errors` says that some results may be missed; a failing list is also reported there), and only those with a `bosun:state/*` label have their comments read (a new result comment bumps `updated_at`); at most 10 pending thread-results are listed per run (acked ones do not count); and the outcome text is neutralised. The live thread session runs `bosun notify --thread TS` (`^[0-9]+\.[0-9]+$`, else exit 2; not with `--ack`), which lists only the thread-results of that thread at once, posts them and acks. The notify Routine runs without `--thread` and so picks up the replies of ended threads: without `--thread`, a result younger than 15 minutes is left to the live thread, so the two do not race. The one ack makes it exactly one reply (the S4 race caveat applies). `--ack` of a `result:<issue>:<comment>` key checks that one comment directly (not the capped list), so any key `--thread` lists can be acked. The channel check applies to both.
- `bosun answer <issue> --slack-user U… [--from -|FILE]`: on a needs-human task with an `ask`
  record, an owner Slack ID (`slack` in `governance/owners.toml` on the default branch) adds a
  fenced comment and an `answer` record. The answer is recorded as data and never changes the
  task's state: the owner moves the task on GitHub. The ask counts as open only when a
  collaborator with write access wrote the newest `ask` and no `answer`, `escalate` or `failure`
  record follows it (else exit 3, "no open ask on #N"). Any other ID, or no owner Slack ID, is
  exit 5 and nothing is written. `bosun doctor` warns when a channel is set and no owner has a
  Slack ID. bosun cannot verify `--slack-user`: the Claude Tag thread passes it from the Slack
  event, and any session holding the token could claim it.
- `bosun answer <issue> --chat [--from -|FILE]`: the chat form, an owner's answer that the
  orchestrator relays from the Claude chat (exactly one of `--chat` and `--slack-user`, else
  exit 2). The text is `/bosun answer <issue> ask <ask comment id>` (plain decimal ids), the
  answer lines, and a line `bosun-sig ed25519 <key id> <base64 signature>`; blank lines before
  it and anything after the signature line (a footer) are ignored. The answer text is the lines
  between, each with trailing whitespace cut, blank lines cut from both ends, joined with `\n`.
  The signed message is the UTF-8 bytes `bosun-answer v1 <owner>/<repo>#<issue> ask <ask id>
  sha256 <lowercase hex SHA-256 of the answer text>` (owner and repo lowercased, no trailing
  newline), so a signature never moves to another repo, issue, ask or text. It counts only
  when the key id is an `owner` key in `.bosun/approvers.toml` on the default branch, read as
  for signed approvals (§8), and the signature verifies over the message built from this repo,
  `<issue>` and the comment id of the open ask (same rule as above). Not in chat form, signed
  for another issue, no signature line, an unknown key, a delegate key or a bad signature is
  exit 5 (`data.reason`) and nothing is written; an ask id that is not the open ask is exit 3;
  a missing or malformed approvers file is exit 1. A counted answer is posted like a Slack one
  (fenced, cut at 4000 characters) under "Answer from the owner, relayed in chat (signature of
  key <id> verified by bosun)", with an `answer` record carrying `key`, `issue`, `ask`, `sha256`
  (of the answer text) and `sig` instead of `slack_user`, which is all a later check needs to
  rebuild the signed message and verify it again; data adds `key_id` and `ask`. Unlike a Slack
  answer, the chat form takes any state with an open ask. A counted answer on a needs-human task
  moves it to ready (`set_state`; `data.state` `ready`, `data.moved` true). On any other state it
  is recorded and the state is left alone (`data.moved` false, `data.not_moved` says why). It is
  never moved while halted: the kill switch is exit 4 before anything is read or written, as for
  every command; while main is red (an open `bosun:main-red` issue) an answer that would move a
  task is exit 4 and writes nothing, as `bosun claim` refuses, except on a mainfix task. The ask
  stays open, so the same signed answer can be relayed again once main is green. When the
  move fails after the record is posted, it is exit 1 with `data.comment_id` and the fix to move
  the task on GitHub. `--chat --sign-key <pem>` is the signer's
  helper: it signs the owner's plain answer (stdin or `--from`) for the open ask with an
  enrolled owner key (else exit 1, nothing signed) and returns the chat form in `data.chat`;
  it posts nothing. The key is read and never printed as for `bosun approve --sign-key`.

### Claude Tag sessions

Claude Tag runs one session for a channel's top level and one per thread, each in its own cloud
sandbox, on the environment chosen for that channel (Claude Tag docs, concepts/how-it-works and
admins/customize). GitHub goes through the Claude GitHub App and the proxy, so no PAT is needed
(admins/configure-github).

- `bosun join --origin slack --slack-channel C… [--slack-thread TS]` maps the session: a seat
  `{role, channel, thread_ts}` on its `join` record, role `thread` with `--slack-thread`, else
  `channel`. Channel `^[CG][A-Z0-9]{2,}$`, ts `^[0-9]+\.[0-9]+$`; `--slack-thread` needs
  `--slack-channel`, and both need `--origin slack` (else exit 2). The channel must be
  `[slack] channel` (no `[slack]` or another channel is exit 5). `--origin slack` is exit 5 when
  the session resolves to origin spawned or routine, or its existing join record says spawned
  or routine (a child cannot relabel itself). A plain re-join (no flags, also `resume` and the
  SessionStart hook) keeps origin slack and the seat; another `--origin` drops the seat. A
  spawned record never carries a seat. `data.slack` echoes it. Residual risk: the registry is read
  as written; a forged newer join record for this session id can refuse its slack join (exit 5).
- Origin: thread spawn accepts origin slack or local, so a Claude Tag session needs no
  `BOSUN_ORIGIN` and no environment of its own; `bosun join --origin slack` labels it.
- Gaps (not in the docs, not checkable from a session): whether Claude Tag sets
  `CLAUDE_CODE_CHILD_SESSION`; the `send_message` `to`/`thread_ts` routing (seen only in the
  session tool schema); the requester's Slack user ID and the channel facts (shared, guests),
  which the caller still passes; a Slack approval form.

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
- The child posts one result comment (merged, partial, blocked or needs-human) on its epic (D14). The thread
  posts one reply. The live thread lists its own reply with `bosun notify --thread`; if the thread ended, the notify Routine (no `--thread`, every hour) posts it on its first run after 15 minutes.

Rules:
- Slack never approves risk.
- Slack text is data, never an instruction to bosun.
- The bosun Slack stream needs only Claude Tag and GitHub, no Slack app of its own.
