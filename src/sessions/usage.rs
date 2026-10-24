//! Usage providers (spec §9, D20): the cost of a session from `claude-sessions` data or a
//! turns-times-price estimate.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::config::Price;
use crate::output::BosunError;

/// Estimated tokens per turn (docs/defaults.md).
pub const INPUT_TOKENS_PER_TURN: f64 = 20_000.0;
pub const OUTPUT_TOKENS_PER_TURN: f64 = 1_500.0;

/// Built-in prices per million tokens, matched by substring of the model name
/// (docs/defaults.md, claude-api skill table cached 2026-10-06).
const BUILTIN: [(&str, f64, f64); 3] = [
    ("haiku", 0.10, 0.50),
    ("sonnet", 2.00, 10.00),
    ("opus", 4.00, 20.00),
];

/// `usage.cost_usd` of a `get_session` result, at the top level or under `session`.
pub fn from_claude_session(v: &Value) -> Result<f64, BosunError> {
    v["usage"]["cost_usd"]
        .as_f64()
        .or_else(|| v["session"]["usage"]["cost_usd"].as_f64())
        .ok_or_else(|| {
            BosunError::error(
                "the session result has no usage.cost_usd",
                "use --provider generic --turns N --model M",
            )
        })
}

fn price_of(model: &str, prices: &BTreeMap<String, Price>) -> Option<(f64, f64)> {
    if let Some(p) = prices.get(model) {
        return Some((p.input_per_mtok, p.output_per_mtok));
    }
    let lower = model.to_lowercase();
    BUILTIN
        .iter()
        .find(|(name, ..)| lower.contains(name))
        .map(|(_, i, o)| (*i, *o))
}

/// `turns` times the estimated tokens per turn times the model's price.
pub fn generic(
    turns: u64,
    model: &str,
    prices: &BTreeMap<String, Price>,
) -> Result<f64, BosunError> {
    let (input, output) = price_of(model, prices).ok_or_else(|| {
        BosunError::error(
            format!("no price for model `{model}`"),
            "add it to [prices] in .bosun/bosun.toml (input_per_mtok, output_per_mtok), or use haiku, sonnet or opus",
        )
    })?;
    let per_turn = INPUT_TOKENS_PER_TURN * input + OUTPUT_TOKENS_PER_TURN * output;
    Ok(turns as f64 * per_turn / 1_000_000.0)
}

/// The cost of one call: its input and output tokens at the model's price.
pub(crate) fn from_tokens(
    input_tokens: u64,
    output_tokens: u64,
    model: &str,
    prices: &BTreeMap<String, Price>,
) -> Result<f64, BosunError> {
    let (input, output) = price_of(model, prices).ok_or_else(|| {
        BosunError::error(
            format!("no price for model `{model}`"),
            "add it to [prices] in .bosun/bosun.toml (input_per_mtok, output_per_mtok), or use haiku, sonnet or opus",
        )
    })?;
    Ok((input_tokens as f64 * input + output_tokens as f64 * output) / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_sessions_reads_both_places() {
        assert_eq!(
            from_claude_session(&json!({"usage": {"cost_usd": 1.5}})).unwrap(),
            1.5
        );
        assert_eq!(
            from_claude_session(&json!({"session": {"usage": {"cost_usd": 2.0}}})).unwrap(),
            2.0
        );
        let e = from_claude_session(&json!({"usage": {}})).unwrap_err();
        assert!(e.fix.contains("--provider generic"));
    }

    #[test]
    fn generic_uses_config_prices_then_builtin() {
        let mut prices = BTreeMap::new();
        prices.insert(
            "my-model".to_string(),
            Price {
                input_per_mtok: 1.0,
                output_per_mtok: 2.0,
            },
        );
        // 10 turns * (20000 * 1 + 1500 * 2) / 1e6
        assert!((generic(10, "my-model", &prices).unwrap() - 0.23).abs() < 1e-9);
        // sonnet: 10 * (20000 * 2 + 1500 * 10) / 1e6
        assert!((generic(10, "claude-sonnet-5-5", &BTreeMap::new()).unwrap() - 0.55).abs() < 1e-9);
        assert!(generic(1, "mystery", &BTreeMap::new()).is_err());
    }
}
