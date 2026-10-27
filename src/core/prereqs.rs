//! Front-loaded human prerequisites (docs/prerequisites.md): the `## Prerequisites` section of a
//! plan, the tasks each prerequisite blocks, the approvals a plan will need at merge (an estimate
//! from touch paths), and the owner checklist (an issue body and a short chat message). Pure.

use crate::core::glob::Glob;
use crate::core::plan::{Plan, PlanError};
use crate::core::risk::{APPROVAL_CODE_PATHS, Approval, CRITICAL_PATHS, Rules};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Prereq {
    pub id: String,
    pub title: String,
    pub who: String,
    pub on: String,
    pub steps: String,
    pub verify: Verify,
    pub blocks: Vec<String>,
    /// Ids of `bosun hold` items (`X1`) this item takes over in the checklist.
    pub replaces: Vec<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Verify {
    Owner,
    Secret { env: Option<String>, name: String },
    Variable { env: Option<String>, name: String },
    Environment { name: String },
    File { path: String },
}

fn re(p: &str) -> regex::Regex {
    regex::Regex::new(p).expect("static regex")
}
static ID: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^H[1-9][0-9]{0,2}$"));
static FIELD: LazyLock<regex::Regex> =
    LazyLock::new(|| re(r"^(who|on|steps|verify|blocks|replaces):"));
static HOLD: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^X[1-9][0-9]{0,2}$"));
static WHO: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^[A-Za-z0-9][A-Za-z0-9._@-]{0,38}$"));
static LIST: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^([-*+]|[0-9]+\.) "));
static BULLET: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^- ([^:\s]+):\s*(\S.*)$"));
static KEY: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^([a-z-]+):\s*(.*)$"));
static NAME: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^[A-Za-z_][A-Za-z0-9_]*$"));
static ENV: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^[A-Za-z0-9_.-]+$"));

impl Verify {
    pub fn parse(v: &str) -> Result<Verify, String> {
        let v = v.trim();
        let (kind, arg) = v.split_once(char::is_whitespace).unwrap_or((v, ""));
        let arg = arg.trim();
        let scoped = |arg: &str| -> Result<(Option<String>, String), String> {
            let (env, name) = match arg.split_once('/') {
                Some((e, n)) => (Some(e), n),
                None => (None, arg),
            };
            if !NAME.is_match(name) || env.is_some_and(|e| !ENV.is_match(e)) {
                return Err(format!("`{arg}` is not NAME or ENV/NAME"));
            }
            if name.to_ascii_uppercase().starts_with("GITHUB_") {
                return Err(format!("`{name}`: GitHub names cannot start with GITHUB_"));
            }
            Ok((env.map(Into::into), name.into()))
        };
        match kind {
            "owner" if arg.is_empty() => Ok(Verify::Owner),
            "secret" => scoped(arg).map(|(env, name)| Verify::Secret { env, name }),
            "variable" => scoped(arg).map(|(env, name)| Verify::Variable { env, name }),
            "environment" if ENV.is_match(arg) => Ok(Verify::Environment { name: arg.into() }),
            "file" if outside(arg) => Err(format!("file `{arg}` must be relative to the repo")),
            "file" if !arg.is_empty() => Ok(Verify::File { path: arg.into() }),
            _ => Err(format!(
                "verify `{v}` is not owner, secret [ENV/]NAME, variable [ENV/]NAME, environment NAME or file PATH"
            )),
        }
    }

    pub fn describe(&self) -> String {
        let sc = |k: &str, env: &Option<String>, name: &String| match env {
            Some(e) => format!("{k} {e}/{name}"),
            None => format!("{k} {name}"),
        };
        match self {
            Verify::Owner => "owner".into(),
            Verify::Secret { env, name } => sc("secret", env, name),
            Verify::Variable { env, name } => sc("variable", env, name),
            Verify::Environment { name } => format!("environment {name}"),
            Verify::File { path } => format!("file {path}"),
        }
    }
}

/// A path that is not plainly inside the repo: absolute, `~`, a backslash or a `..` part.
fn outside(p: &str) -> bool {
    p.starts_with(['/', '~']) || p.contains('\\') || p.split('/').any(|c| c == "..")
}

macro_rules! bad {
    ($v:expr, $n:expr; $fix:expr => $($m:tt)*) => {
        $v.push(PlanError { line: $n, message: format!($($m)*), fix: $fix.into() })
    };
}

fn csv(v: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let parts = v.split(',').map(str::trim);
    parts
        .filter(|s| !s.is_empty() && seen.insert(*s))
        .map(Into::into)
        .collect()
}

/// A bullet being read: its fields so far, each with its line.
struct Open {
    id: String,
    title: String,
    line: usize,
    kv: HashMap<String, (String, usize)>,
}

fn finish(o: Open, plan: &Plan, out: &mut Vec<Prereq>, errs: &mut Vec<PlanError>) {
    let id = o.id.clone();
    let get = |k: &str| o.kv.get(k).filter(|(v, _)| !v.is_empty());
    let at = |k: &str| o.kv.get(k).map_or(o.line, |(_, l)| *l);
    let val = |k: &str| get(k).map_or("", |(v, _)| v.as_str());
    let on = val("on");
    if on.is_empty() {
        bad!(errs, o.line; "write `on: phone` or `on: computer`" => "{id} has no `on:`");
    } else if !matches!(on, "phone" | "computer") {
        bad!(errs, at("on"); "write `on: phone` or `on: computer`" => "{id}: `on:` is `{on}`, not phone or computer");
    }
    let texts = [
        ("title", o.title.as_str()),
        ("steps", val("steps")),
        ("who", val("who")),
    ];
    for (k, v) in texts {
        if let Some(why) = unsafe_text(v) {
            bad!(errs, at(k); "write plain text" => "{id}: `{k}` contains {why}");
        }
    }
    let who = get("who").map_or("owner", |(v, _)| v.as_str());
    if !WHO.is_match(who) {
        bad!(errs, at("who"); "write a plain name like owner or ada" => "{id}: `who:` is not a plain name");
    }
    if val("steps").is_empty() {
        bad!(errs, o.line; "write the steps on one line, `steps: ...`" => "{id} has no `steps:`");
    }
    let blocks = csv(val("blocks"));
    if blocks.is_empty() {
        bad!(errs, o.line; "list what waits for it, e.g. `blocks: M1/T2, M2`" => "{id} has no `blocks:`");
    }
    for b in &blocks {
        let (item, task) = b
            .split_once('/')
            .map_or((b.as_str(), None), |(i, t)| (i, Some(t)));
        let found = plan.items.iter().find(|i| i.id == item);
        if found.is_none_or(|i| task.is_some_and(|t| !i.tasks.iter().any(|x| x.id == t))) {
            bad!(errs, at("blocks"); "name an item like M1 or a task like M1/T2" => "{id} blocks `{b}`, which is not an item or task of the plan");
        }
    }
    if o.kv.get("verify").is_some_and(|(v, _)| v.is_empty()) {
        bad!(errs, at("verify"); "write how bosun checks it, or leave the line out for owner" => "{id} has an empty `verify:`");
    }
    let verify = Verify::parse(get("verify").map_or("owner", |(v, _)| v)).unwrap_or_else(|e| {
        bad!(errs, at("verify"); "use owner, secret NAME, variable NAME, environment NAME or file PATH" => "{id}: {e}");
        Verify::Owner
    });
    let replaces = csv(val("replaces"));
    if let Some(x) = replaces.iter().find(|x| !HOLD.is_match(x)) {
        bad!(errs, at("replaces"); "name `bosun hold` items, e.g. `replaces: X1`" => "{id} replaces `{x}`, which is not an X item");
    }
    out.push(Prereq {
        title: o.title.clone(),
        who: who.into(),
        on: on.into(),
        steps: val("steps").into(),
        verify,
        blocks,
        replaces,
        line: o.line,
        id,
    });
}

/// Why `t` cannot go into the checklist issue (a comment hides text or forges a marker; bidi lies).
fn unsafe_text(t: &str) -> Option<&'static str> {
    let bidi = |c: char| matches!(c, '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    let control = t.chars().any(|c| c.is_control() || bidi(c));
    let html = t.contains("<!--") || t.contains("-->");
    (html.then_some("an HTML comment marker")).or(control.then_some("a control or bidi character"))
}

/// A heading (any level) like a misspelt `## Prerequisites` (`## Pre-requisites`).
fn near_miss(line: &str) -> bool {
    let h = line.trim_start_matches('#').trim().to_lowercase();
    h.starts_with("pre") && h.contains("requisite") && line != "## Prerequisites"
}

pub fn parse(text: &str, plan: &Plan) -> Result<Vec<Prereq>, Vec<PlanError>> {
    let (mut out, mut errs) = (Vec::new(), Vec::new());
    let (mut in_sec, mut seen, mut fence): (bool, bool, Option<&str>) = (false, false, None);
    let mut cur: Option<Open> = None;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for (i, raw) in text.lines().enumerate() {
        let (n, line) = (i + 1, raw.trim_end());
        let marker = ["```", "~~~"]
            .into_iter()
            .find(|m| line.trim_start().starts_with(m));
        if let Some(m) = marker.filter(|m| fence.is_none_or(|f| f == *m)) {
            fence = if fence.is_some() { None } else { Some(m) };
            continue;
        }
        if fence.is_some() || line.trim().is_empty() {
            continue;
        }
        let indented = line.starts_with([' ', '\t']);
        if !indented && let Some(o) = cur.take() {
            finish(o, plan, &mut out, &mut errs);
        }
        if line.starts_with('#') && !line.starts_with("## ") && near_miss(line) {
            bad!(errs, n; "write the heading as `## Prerequisites`" => "`{line}` is not `## Prerequisites`");
            continue;
        }
        if line.starts_with("## ") {
            in_sec = line == "## Prerequisites";
            if near_miss(line) {
                bad!(errs, n; "write the heading as `## Prerequisites`" => "`{line}` is not `## Prerequisites`");
            } else if in_sec && std::mem::replace(&mut seen, true) {
                bad!(errs, n; "keep one `## Prerequisites` section" => "a second `## Prerequisites` section");
            }
            continue;
        }
        if !in_sec {
            continue;
        }
        if indented {
            let Some(o) = cur.as_mut() else {
                bad!(errs, n; "put the key line under a `- H1: title` bullet" => "indented line before any prerequisite");
                continue;
            };
            let Some(c) = KEY.captures(line.trim()) else {
                bad!(errs, n; "write fields as `  key: value`, each on one line" => "expected an indented `key: value` line");
                continue;
            };
            let k = c[1].to_string();
            if !["who", "on", "steps", "verify", "blocks", "replaces"].contains(&k.as_str()) {
                bad!(errs, n; "use who, on, steps, verify, blocks or replaces" => "unknown prerequisite key `{k}`");
            } else if o.kv.insert(k.clone(), (c[2].trim().into(), n)).is_some() {
                bad!(errs, n; "keep one line per key" => "`{k}:` is given twice for {}", o.id);
            }
        } else if FIELD.is_match(line) {
            bad!(errs, n; "indent field lines under their `- H1: title` bullet" => "field line `{line}` is not indented");
        } else if LIST.is_match(line) {
            match BULLET.captures(line) {
                Some(c) if ID.is_match(&c[1]) => {
                    if out.iter().any(|p| p.id == c[1]) {
                        bad!(errs, n; "give every prerequisite its own id" => "prerequisite id `{}` is used twice", &c[1]);
                    }
                    cur = Some(Open {
                        id: c[1].into(),
                        title: c[2].trim().into(),
                        line: n,
                        kv: HashMap::new(),
                    });
                }
                _ => {
                    bad!(errs, n; "write the bullet as `- H1: title` (id H1 to H999)" => "`{line}` is not `- H<n>: <title>`")
                }
            }
        }
    }
    if let Some(o) = cur.take() {
        finish(o, plan, &mut out, &mut errs);
    }
    if errs.is_empty() { Ok(out) } else { Err(errs) }
}

/// Ids of the prerequisites that block this task (listed as `<item>/<task>` or as `<item>`).
pub fn blocked_by<'a>(prereqs: &'a [Prereq], item: &str, task: &str) -> Vec<&'a str> {
    let full = format!("{item}/{task}");
    let hit = |b: &String| *b == full || b == item;
    prereqs
        .iter()
        .filter(|p| p.blocks.iter().any(hit))
        .map(|p| p.id.as_str())
        .collect()
}

/// A task whose PR will need an approval at merge: `needs` is `owner` or `delegate`, `batch`
/// its dependency batch (1 first).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApprovalNeed {
    pub task: String,
    pub needs: String,
    pub why: Vec<String>,
    pub batch: usize,
}

/// Dependency level of `item/task`: 1 + the deepest task it depends on, directly or through
/// the items its item depends on.
fn level(key: &str, plan: &Plan, memo: &mut HashMap<String, usize>) -> usize {
    if let Some(&l) = memo.get(key) {
        return l;
    }
    // Guard: a cycle through item and task depends (plan::parse checks each graph alone) gets
    // an arbitrary batch instead of looping.
    memo.insert(key.into(), 1);
    let (iid, tid) = key.split_once('/').unwrap_or((key, ""));
    let mut deps: Vec<String> = vec![];
    if let Some(it) = plan.items.iter().find(|i| i.id == iid) {
        if let Some(t) = it.tasks.iter().find(|t| t.id == tid) {
            deps.extend(t.depends.iter().cloned());
        }
        for d in it.depends.iter().filter(|d| *d != "seed") {
            for x in plan.items.iter().filter(|x| x.id == *d) {
                deps.extend(x.tasks.iter().map(|t| t.plan_id(d)));
            }
        }
    }
    let l = 1 + deps.iter().map(|d| level(d, plan, memo)).max().unwrap_or(0);
    memo.insert(key.into(), l);
    l
}

/// The tasks of a plan whose PR will need an approval, in plan order, in dependency batches.
pub fn approvals(plan: &Plan, rules: &Rules, bosun_code: bool) -> Vec<ApprovalNeed> {
    let glob = |g: &&str| Glob::parse(g).expect("built-in critical glob");
    let mut floor: Vec<Glob> = CRITICAL_PATHS.iter().map(glob).collect();
    if bosun_code {
        floor.extend(APPROVAL_CODE_PATHS.iter().map(glob));
    }
    let (mut out, mut memo) = (Vec::new(), HashMap::new());
    for it in &plan.items {
        for t in &it.tasks {
            let touch: Vec<Glob> = t.touch.iter().filter_map(|g| Glob::parse(g).ok()).collect();
            let hits: Vec<_> = rules
                .rules
                .iter()
                .filter(|r| touch.iter().any(|g| r.paths.iter().any(|p| g.overlaps(p))))
                .collect();
            let crit: Vec<String> = touch
                .iter()
                .filter(|g| floor.iter().any(|f| g.overlaps(f)))
                .map(|g| format!("critical: {}", g.as_str()))
                .collect();
            let any = |a| hits.iter().any(|r| r.approval == a);
            let (high, d) = (t.risk == "high", any(Approval::Delegate));
            let tv = any(Approval::TwoVerdicts);
            // As risk-check: the floor only turns delegate matches into owner ones, and a task
            // the plan marks `risk: high` is expected to need the owner whatever it touches.
            let floor_hit = d && !crit.is_empty();
            let needs = if any(Approval::Owner) || (d && tv) || floor_hit || high {
                "owner"
            } else if d {
                "delegate"
            } else {
                continue;
            };
            let mut why: Vec<String> = hits.iter().map(|r| r.id.clone()).collect();
            if floor_hit {
                why.extend(crit);
            }
            why.extend(high.then(|| "risk: high".to_string()));
            let task = t.plan_id(&it.id);
            let batch = level(&task, plan, &mut memo);
            out.push(ApprovalNeed {
                task,
                needs: needs.into(),
                why,
                batch,
            });
        }
    }
    let levels: BTreeSet<usize> = out.iter().map(|a| a.batch).collect();
    for a in &mut out {
        a.batch = levels.iter().position(|l| *l == a.batch).unwrap_or(0) + 1;
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckItem {
    pub id: String,
    pub title: String,
    pub who: String,
    pub on: String,
    pub steps: String,
    pub verify: String,
    /// bosun checks it through the API (false for `owner`, and after a 403).
    pub api: bool,
    /// Done: verified through the API, or ticked when `api` is false.
    pub done: bool,
    pub blocks: Vec<String>,
}

impl From<&Prereq> for CheckItem {
    fn from(p: &Prereq) -> Self {
        CheckItem {
            id: p.id.clone(),
            title: p.title.clone(),
            who: p.who.clone(),
            on: p.on.clone(),
            steps: p.steps.clone(),
            verify: p.verify.describe(),
            api: p.verify != Verify::Owner,
            done: false,
            blocks: p.blocks.clone(),
        }
    }
}

/// Open computer items first, then open phone items, then done ones; stable otherwise.
pub fn sort(items: &mut [CheckItem]) {
    items.sort_by_key(|i| {
        if i.done {
            2
        } else {
            u8::from(i.on != "computer")
        }
    });
}

pub fn issue_marker(plan_path: &str) -> String {
    format!("<!-- bosun:prereqs {plan_path} -->")
}

type Group = (&'static str, fn(&CheckItem) -> bool);
const GROUPS: [Group; 3] = [
    ("Needs a computer", |i| !i.done && i.on == "computer"),
    ("On your phone", |i| !i.done && i.on != "computer"),
    ("Done", |i| i.done),
];

pub fn render_issue(
    plan_title: &str,
    plan_path: &str,
    items: &[CheckItem],
    approvals: &[ApprovalNeed],
) -> String {
    let mut s = format!(
        "{}\nOwner checklist for **{plan_title}**. Tick a box when done; bosun checks what it can and ticks it itself.\n",
        issue_marker(plan_path)
    );
    for (head, f) in GROUPS {
        let rows: Vec<_> = items.iter().filter(|i| f(i)).collect();
        if rows.is_empty() {
            continue;
        }
        s += &format!("### {head}\n");
        for i in rows {
            let x = if i.done { 'x' } else { ' ' };
            let (title, steps, verify) = (safe(&i.title), safe(&i.steps), safe(&i.verify));
            let who = by(i);
            let blocks = i.blocks.join(",");
            let mark = match (i.api, i.done) {
                (false, _) => "owner",
                (true, false) => "api",
                (true, true) => "api ok",
            };
            s += &format!(
                "- [{x}] **{}**{who} {title} \u{2014} {steps} _(verify: {verify}; blocks {blocks})_ <!-- bosun:prereq {} blocks={blocks} verify={mark} -->\n",
                i.id, i.id
            );
        }
    }
    if !approvals.is_empty() {
        s += "### Approvals at merge\n";
        let last = approvals.iter().map(|a| a.batch).max().unwrap_or(0);
        for b in 1..=last {
            let row: Vec<_> = approvals
                .iter()
                .filter(|a| a.batch == b)
                .map(|a| format!("{} ({})", a.task, a.needs))
                .collect();
            s += &format!("Batch {b}: {}\n", row.join(", "));
        }
    }
    s
}

/// ` (by bob)` when someone other than the owner does it.
fn by(i: &CheckItem) -> String {
    if i.who == "owner" {
        String::new()
    } else {
        format!(" (by {})", safe(&i.who))
    }
}

/// User text made safe for one checklist line: no HTML comment, no line break.
fn safe(t: &str) -> String {
    let t = t
        .replace("<!--", "<!-\u{200b}-")
        .replace("-->", "-\u{200b}->");
    t.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// One box of a checklist issue, as bosun wrote it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tick {
    pub id: String,
    pub ticked: bool,
    pub api: bool,
    /// bosun verified it through the API (`verify=api ok`).
    pub ok: bool,
    pub blocks: Vec<String>,
}

impl Tick {
    /// An API item is done only when bosun verified it; an owner item when its box is ticked.
    pub fn done(&self) -> bool {
        if self.api { self.ok } else { self.ticked }
    }
}

/// The boxes of a checklist issue body, in order. Only lines that end in bosun's marker count.
pub fn read_issue(body: &str) -> Vec<Tick> {
    let mut out = Vec::new();
    for l in body.lines().map(str::trim) {
        let ticked = l.starts_with("- [x]") || l.starts_with("- [X]");
        let Some((_, m)) = l.rsplit_once("<!-- bosun:prereq ") else {
            continue;
        };
        let Some(m) = m
            .strip_suffix(" -->")
            .filter(|_| ticked || l.starts_with("- [ ]"))
        else {
            continue;
        };
        let (m, verify) = m.split_once(" verify=").unwrap_or((m, "owner"));
        let (id, blocks) = m.split_once(" blocks=").unwrap_or((m, ""));
        out.push(Tick {
            id: id.trim().into(),
            ticked,
            api: verify.starts_with("api"),
            ok: verify == "api ok",
            blocks: csv(blocks),
        });
    }
    out
}

/// At most `n` characters, with `…` when cut.
fn cut(t: &str, n: usize) -> String {
    if t.chars().count() <= n {
        t.into()
    } else {
        t.chars().take(n - 1).collect::<String>() + "\u{2026}"
    }
}

pub fn message(
    plan_title: &str,
    items: &[CheckItem],
    approvals: &[ApprovalNeed],
    issue_url: Option<&str>,
) -> String {
    let mut open: Vec<&CheckItem> = items.iter().filter(|i| !i.done).collect();
    open.sort_by_key(|i| u8::from(i.on != "computer"));
    let computer = open.iter().filter(|i| i.on == "computer").count();
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut lines = vec![if open.is_empty() {
        format!("{plan_title}: nothing for you to prepare.")
    } else {
        let steps = plural(open.len(), "step", "steps");
        format!("{plan_title}: {steps} before agents can finish ({computer} on a computer).")
    }];
    for i in open.iter().take(15) {
        let (id, on, who) = (&i.id, &i.on, by(i));
        lines.push(format!(
            "- {id} ({on}){who}: {}: {}",
            cut(&i.title, 60),
            cut(&i.steps, 80)
        ));
    }
    if open.len() > 15 {
        lines.push(format!(
            "- \u{2026} and {} more (see the issue)",
            open.len() - 15
        ));
    }
    if !approvals.is_empty() {
        let batches = approvals
            .iter()
            .map(|a| a.batch)
            .collect::<BTreeSet<_>>()
            .len();
        let owner: Vec<_> = approvals
            .iter()
            .filter(|a| a.needs == "owner")
            .map(|a| a.task.as_str())
            .collect();
        let mut parts = Vec::new();
        if !owner.is_empty() {
            let shown = owner.iter().take(8).copied().collect::<Vec<_>>().join(", ");
            let more = if owner.len() > 8 { ", \u{2026}" } else { "" };
            parts.push(format!(
                "{} need you: {shown}{more}",
                plural(owner.len(), "PR", "PRs")
            ));
        }
        let delegate = approvals.len() - owner.len();
        if delegate > 0 {
            parts.push(format!("{delegate} by a delegate"));
        }
        let batches = plural(batches, "approval batch", "approval batches");
        lines.push(format!("Later: {batches} ({}).", parts.join("; ")));
    }
    lines.extend(issue_url.map(|u| format!("Tick them off here: {u}")));
    lines.join("\n")
}
