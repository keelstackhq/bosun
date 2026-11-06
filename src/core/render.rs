//! A tiny template renderer for the subset of Jinja the bosun channel uses: `{{ var }}`,
//! `{% if var %}`, `{% if "s" in pieces %}`, `{% else %}`, `{% endif %}` (nested) and
//! `{% raw %}...{% endraw %}`. Anything else is an error. No I/O.

use std::collections::BTreeMap;

/// The variables of one render: named strings (empty is false) and the `pieces` list.
#[derive(Debug, Clone, Default)]
pub struct Vars {
    pub vars: BTreeMap<String, String>,
    pub pieces: Vec<String>,
}

impl Vars {
    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        self.vars.insert(name.to_string(), value.into());
    }
}

fn lookup<'a>(vars: &'a Vars, name: &str) -> Result<&'a str, String> {
    vars.vars
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("unknown variable {name:?}"))
}

fn condition(expr: &str, vars: &Vars) -> Result<bool, String> {
    if let Some((lit, list)) = expr.split_once(" in ") {
        let lit = lit.trim();
        let name = lit
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or_else(|| format!("unsupported condition {expr:?}"))?;
        if list.trim() != "pieces" {
            return Err(format!("unsupported condition {expr:?}"));
        }
        return Ok(vars.pieces.iter().any(|p| p == name));
    }
    Ok(!lookup(vars, expr.trim())?.is_empty())
}

/// Render `src`. Errors name the first unsupported tag, unknown variable or unbalanced block.
pub fn render(src: &str, vars: &Vars) -> Result<String, String> {
    let mut out = String::new();
    let mut stack: Vec<bool> = Vec::new();
    let mut rest = src;
    while let Some(at) = rest.find("{%").into_iter().chain(rest.find("{{")).min() {
        let active = stack.iter().all(|c| *c);
        if active {
            out.push_str(&rest[..at]);
        }
        let (close, is_tag) = if rest[at..].starts_with("{%") {
            ("%}", true)
        } else {
            ("}}", false)
        };
        let end = rest[at..]
            .find(close)
            .ok_or_else(|| format!("unclosed {} tag", &rest[at..at + 2]))?
            + at;
        let inner = rest[at + 2..end].trim();
        rest = &rest[end + 2..];
        if !is_tag {
            if active {
                out.push_str(lookup(vars, inner)?);
            }
            continue;
        }
        if inner == "raw" {
            let mut scan = 0;
            let raw_end = loop {
                let open = rest[scan..]
                    .find("{%")
                    .ok_or("{% raw %} without {% endraw %}")?
                    + scan;
                let close = rest[open..].find("%}").ok_or("unclosed tag")? + open;
                if rest[open + 2..close].trim() == "endraw" {
                    break (open, close);
                }
                scan = open + 2;
            };
            if active {
                out.push_str(&rest[..raw_end.0]);
            }
            rest = &rest[raw_end.1 + 2..];
        } else if let Some(expr) = inner.strip_prefix("if ") {
            // A false parent makes the child false too; its value is never read.
            stack.push(if active {
                condition(expr, vars)?
            } else {
                false
            });
        } else if inner == "else" {
            let top = stack.pop().ok_or("{% else %} without {% if %}")?;
            let parent = stack.iter().all(|c| *c);
            stack.push(parent && !top);
        } else if inner == "endif" {
            stack.pop().ok_or("{% endif %} without {% if %}")?;
        } else {
            return Err(format!("unsupported tag {{% {inner} %}}"));
        }
    }
    if !stack.is_empty() {
        return Err("{% if %} without {% endif %}".into());
    }
    if stack.iter().all(|c| *c) {
        out.push_str(rest);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pieces: &[&str]) -> Vars {
        let mut v = Vars {
            pieces: pieces.iter().map(|s| s.to_string()).collect(),
            ..Vars::default()
        };
        v.set("project", "app");
        v.set("empty", "");
        v.set("template.version", "1.2.3");
        v
    }

    #[test]
    fn substitutes_variables() {
        let r = render("a {{ project }} v{{ template.version }}", &vars(&[]));
        assert_eq!(r.unwrap(), "a app v1.2.3");
    }

    #[test]
    fn if_else_on_variable_and_pieces() {
        let t = "{% if empty %}A{% else %}B{% endif %}{% if \"rust\" in pieces %}R{% endif %}";
        assert_eq!(render(t, &vars(&["rust"])).unwrap(), "BR");
        assert_eq!(render(t, &vars(&[])).unwrap(), "B");
    }

    #[test]
    fn nested_blocks_and_else_inside_false_parent() {
        let t = "{% if empty %}{% if project %}x{% else %}y{% endif %}{% else %}z{% endif %}";
        assert_eq!(render(t, &vars(&[])).unwrap(), "z");
        let t = "{% if project %}{% if empty %}x{% else %}y{% endif %}{% endif %}";
        assert_eq!(render(t, &vars(&[])).unwrap(), "y");
    }

    #[test]
    fn raw_passes_through() {
        let t = "{% raw %}${{ github.token }} {% if x %}{% endraw %}{{ project }}";
        assert_eq!(
            render(t, &vars(&[])).unwrap(),
            "${{ github.token }} {% if x %}app"
        );
    }

    #[test]
    fn errors() {
        assert!(render("{% for x in y %}", &vars(&[])).is_err());
        assert!(render("{{ nope }}", &vars(&[])).is_err());
        assert!(render("{% if project %}", &vars(&[])).is_err());
        assert!(render("{% endif %}", &vars(&[])).is_err());
        assert!(render("{% raw %}x", &vars(&[])).is_err());
    }
}
