//! The strict check of an envelope against `bosun schema`: every `data` key of an ok result must
//! be in its command's schema. `one_object` runs it on every envelope the tests see.

use bosun::schema::{ERROR, NEXT, Ty, data_of, parse};
use serde_json::Value;

/// Strict check of `v` against `t`: every key of an object must be listed (unless `..`).
pub fn check(t: &Ty, v: &Value, at: &str) -> Result<(), String> {
    let ok = match (t, v) {
        (Ty::Any, _) | (Ty::Null, Value::Null) | (Ty::Bool, Value::Bool(_)) => true,
        (Ty::Int, Value::Number(n)) => n.is_i64() || n.is_u64(),
        (Ty::Num, Value::Number(_)) | (Ty::Str, Value::String(_)) => true,
        (Ty::Lit(l), Value::String(s)) => l == s,
        (Ty::Arr(i), Value::Array(a)) => {
            for (n, x) in a.iter().enumerate() {
                check(i, x, &format!("{at}[{n}]"))?;
            }
            true
        }
        (Ty::Map(t), Value::Object(m)) => {
            for (k, x) in m {
                check(t, x, &format!("{at}.{k}"))?;
            }
            true
        }
        (Ty::Obj(keys, open), Value::Object(m)) => {
            for (k, req, t) in keys {
                match m.get(k) {
                    Some(x) => check(t, x, &format!("{at}.{k}"))?,
                    None if *req => return Err(format!("{at}.{k} is missing")),
                    None => {}
                }
            }
            if let Some(k) = m.keys().find(|k| !open && !keys.iter().any(|x| &x.0 == *k)) {
                return Err(format!("{at}.{k} is not in the schema"));
            }
            true
        }
        (Ty::Union(alts), _) => {
            let errs: Vec<String> = alts.iter().filter_map(|a| check(a, v, at).err()).collect();
            if errs.len() == alts.len() {
                return Err(errs.join("; or "));
            }
            true
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{at} is {v}, not {t:?}"))
    }
}

/// Check one envelope strictly: the envelope keys, and `data` of an `ok` result against its
/// command's schema. A command without a schema (a usage error before the subcommand) checks
/// the envelope only.
pub fn check_envelope(v: &Value) -> Result<(), String> {
    let shape = format!(
        "{{version: int, command: str, ok: bool, exit_code: int, data: {{..}}, error: null | {ERROR}, next: {NEXT}}}"
    );
    check(&parse(&shape)?, v, "")?;
    let (ok, code) = (v["ok"] == true, v["exit_code"].as_u64().unwrap_or(9));
    if v["version"] != 1 || code > 6 || ok != (code == 0) || ok != v["error"].is_null() {
        return Err(format!("version, ok, exit_code and error disagree: {v}"));
    }
    if !ok {
        if v["error"]["fix"]
            .as_str()
            .is_none_or(|f| f.trim().is_empty())
        {
            return Err("error.fix is empty".into());
        }
        return Ok(());
    }
    match v["command"].as_str().and_then(data_of) {
        Some(d) => check(&parse(d)?, &v["data"], "data"),
        None => Ok(()),
    }
}
