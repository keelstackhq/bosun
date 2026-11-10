//! `bosun schema --format ts`: the contract as TypeScript declarations for Code Mode harnesses.

use super::{Ty, data_of, names, parse};

fn ty(t: &Ty) -> String {
    match t {
        Ty::Any => "unknown".into(),
        Ty::Null => "null".into(),
        Ty::Bool => "boolean".into(),
        Ty::Int | Ty::Num => "number".into(),
        Ty::Str => "string".into(),
        Ty::Lit(l) => serde_json::to_string(l).unwrap_or_default(),
        Ty::Arr(i) if matches!(**i, Ty::Union(_)) => format!("({})[]", ty(i)),
        Ty::Arr(i) => format!("{}[]", ty(i)),
        Ty::Map(v) => format!("Record<string, {}>", ty(v)),
        Ty::Union(alts) => alts.iter().map(ty).collect::<Vec<_>>().join(" | "),
        Ty::Obj(keys, _) if keys.is_empty() => "Record<string, unknown>".into(),
        Ty::Obj(keys, open) => {
            let mut parts: Vec<String> = keys
                .iter()
                .map(|(k, req, t)| {
                    let k = if k.contains('-') {
                        format!("\"{k}\"")
                    } else {
                        k.clone()
                    };
                    format!("{k}{}: {}", if *req { "" } else { "?" }, ty(t))
                })
                .collect();
            if *open {
                parts.push("[key: string]: unknown".into());
            }
            format!("{{ {} }}", parts.join("; "))
        }
    }
}

/// `verdict post` -> (`verdictPost`, `VerdictPostData`).
fn idents(command: &str) -> (String, String) {
    let words: Vec<&str> = command
        .split([' ', '-'])
        .filter(|w| !w.is_empty())
        .collect();
    let cap = |w: &str| w[..1].to_uppercase() + &w[1..];
    let camel = words[0].to_string() + &words[1..].iter().map(|w| cap(w)).collect::<String>();
    let pascal = words.iter().map(|w| cap(w)).collect::<String>() + "Data";
    (camel, pascal)
}

const HEAD: &str = r#"// bosun's output contract. Every call prints one Envelope on stdout; branch on exit_code
// (0 ok, 1 error, 2 usage, 3 conflict, 4 halted, 5 needs-human, 6 limit) and do what error.fix says.
declare namespace bosun {
  interface BosunError { kind: "error" | "usage" | "conflict" | "halted" | "needs-human" | "limit" | "upstream"; message: string; fix: string }
  /** A tool call the session must make (D29). */
  interface Next { tool: string; args: unknown }
  type Envelope<C extends string, D> = { version: 1; command: C; next: Next[] } & (
    | { ok: true; exit_code: 0; data: D; error: null }
    | { ok: false; exit_code: 1 | 2 | 3 | 4 | 5 | 6; data: Record<string, unknown>; error: BosunError; next: [] }
  );
"#;

/// The declarations for one command, or every command.
pub fn declarations(command: Option<&str>) -> String {
    let mut out = HEAD.to_string();
    for name in names().filter(|n| command.is_none_or(|c| c == *n)) {
        let notation = data_of(name).unwrap_or("{..}");
        let data = parse(notation).map(|t| ty(&t)).unwrap_or("unknown".into());
        let (f, d) = idents(name);
        out.push_str(&format!(
            "  type {d} = {data};\n  /** `bosun {name} ...args` */\n  function {f}(...args: string[]): Promise<Envelope<\"{name}\", {d}>>;\n"
        ));
    }
    out + "}\n"
}
