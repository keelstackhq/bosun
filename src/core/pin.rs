//! Pure logic of `bosun pin`: strict X.Y.Z versions and rewriting the `KEY=value` pin lines of
//! `docs/setup-script.sh` (and its channel template).

/// `X.Y.Z` of plain digits, nothing else (no `v`, no pre-release, no build metadata).
pub fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.split('.');
    let mut part = || {
        let p = it.next()?;
        (!p.is_empty()
            && p.bytes().all(|b| b.is_ascii_digit())
            && (p == "0" || !p.starts_with('0')))
        .then(|| p.parse().ok())
        .flatten()
    };
    let v = (part()?, part()?, part()?);
    it.next().is_none().then_some(v)
}

/// The base64 part of an npm `sha512-<b64>` integrity string.
pub fn sha512_of(integrity: &str) -> Option<&str> {
    integrity.strip_prefix("sha512-").filter(|b| !b.is_empty())
}

/// The value of the one line `KEY=value`; `None` when there are zero or several.
pub fn get_line<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut hits = text
        .lines()
        .filter_map(|l| l.strip_prefix(key)?.strip_prefix('='));
    let v = hits.next()?;
    hits.next().is_none().then_some(v)
}

/// `text` with the one line `KEY=...` replaced by `KEY=value`; `Err` when it is not there exactly once.
pub fn set_line(text: &str, key: &str, value: &str) -> Result<String, String> {
    get_line(text, key).ok_or_else(|| format!("{key}= must appear exactly once"))?;
    let mut out = String::with_capacity(text.len());
    for l in text.split_inclusive('\n') {
        if l.strip_prefix(key).is_some_and(|r| r.starts_with('=')) {
            out.push_str(&format!("{key}={value}"));
            if l.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(l);
        }
    }
    Ok(out)
}

/// Pins of one tool: version and the sha512 (base64) of the launcher and both linux tarballs.
pub struct Pins {
    pub version: String,
    pub launcher: String,
    pub x64: String,
    pub arm64: String,
}

/// The four line keys of `tool` (`splice` -> `SPLICE_VERSION`, ...).
pub fn keys(tool: &str) -> [String; 4] {
    let u = tool.to_uppercase();
    [
        format!("{u}_VERSION"),
        format!("{u}_SHA512"),
        format!("{u}_LINUX_X64_SHA512"),
        format!("{u}_LINUX_ARM64_SHA512"),
    ]
}

/// Rewrite the four pin lines of `tool` in `text`.
pub fn apply(text: &str, tool: &str, p: &Pins) -> Result<String, String> {
    let [kv, kl, kx, ka] = keys(tool);
    let t = set_line(text, &kv, &p.version)?;
    let t = set_line(&t, &kl, &p.launcher)?;
    let t = set_line(&t, &kx, &p.x64)?;
    set_line(&t, &ka, &p.arm64)
}
