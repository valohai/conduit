use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock};

use serde_json::Value;

use crate::Provider;
use crate::config::ModelPricing;

const FALLBACK_PRICING_JSON: &str = include_str!("../pricing.json");

static FALLBACK_PRICING: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(FALLBACK_PRICING_JSON).expect("invalid fallback pricing.json")
});

// NB: process-wide and set once at startup, so storage and the dashboard need no plumbing;
// pass it through `Storages` if one process ever needs two price tables
static CUSTOM_PRICING: OnceLock<Value> = OnceLock::new();

/// Use the `[pricing]` table of `conduit.toml` before the built-in one. Only the first call counts.
pub fn set_custom_pricing(pricing: &HashMap<String, ModelPricing>) {
    let table = serde_json::to_value(pricing).expect("prices serialize to JSON");
    let _ = CUSTOM_PRICING.set(table);
}

pub fn estimate_cost(provider: Provider, model: &str, usage: &Value) -> Option<f64> {
    estimate_cost_with(CUSTOM_PRICING.get(), provider, model, usage)
}

#[rustfmt::skip]
fn estimate_cost_with(custom: Option<&Value>, provider: Provider, model: &str, usage: &Value) -> Option<f64> {
    let rates = lookup_rates(custom, provider, model)?;

    let input_tokens = usage
        // OpenAI Responses, Anthropic Messages
        .get("input_tokens")
        // OpenAI Chat Completions
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(|v| v.as_u64());

    // NB: in general, internal "reasoning tokens" are already included in
    //     `output_tokens`, revise if it ain't so on a future API

    let output_tokens = usage
        // OpenAI Responses, Anthropic Messages
        .get("output_tokens")
        // OpenAI Chat Completions
        .or_else(|| usage.get("completion_tokens"))
        .and_then(|v| v.as_u64());

    if input_tokens.is_none() && output_tokens.is_none() {
        return None;
    }

    let input_tokens = input_tokens.unwrap_or(0);
    let output_tokens = output_tokens.unwrap_or(0);

    let cache_read_tokens = usage
        // Anthropic Messages
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let cache_write_tokens = usage
        // Anthropic Messages
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // without an explicit TTL the API writes to the 5-minute cache, and a stream's TTL split
    // covers only the writes before the first output token; server tools write 5-minute ones after it
    let split = usage.get("cache_creation").filter(|s| s.is_object());
    let split_tokens = |key: &str| split.and_then(|s| s.get(key)).and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_write_1h_tokens = split_tokens("ephemeral_1h_input_tokens");
    let cache_write_5m_tokens = split_tokens("ephemeral_5m_input_tokens")
        .max(cache_write_tokens.saturating_sub(cache_write_1h_tokens));

    let mut cost =
          rates.input          * input_tokens          as f64 / 1_000_000.0
        + rates.output         * output_tokens         as f64 / 1_000_000.0
        + rates.cached_input   * cache_read_tokens     as f64 / 1_000_000.0
        + rates.cache_write_5m * cache_write_5m_tokens as f64 / 1_000_000.0
        + rates.cache_write_1h * cache_write_1h_tokens as f64 / 1_000_000.0
    ;

    if provider == Provider::OpenAI {
        // TODO: https://developers.openai.com/api/docs/pricing (under "Regional processing")
        // > [Data residency] endpoints are charged a 10% uplift for gpt-5.4, gpt-5.4-mini, gpt-5.4-nano, and gpt-5.4-pro.
        // how to detect?

        // TODO: https://developers.openai.com/api/docs/guides/priority-processing
        // by using `service_tier="priority"` on the calls
        // how to detect?

        // TODO: flex pricing is 50% off https://developers.openai.com/api/docs/guides/flex-processing
        // by using `service_tier="flex"` on the calls
        // how to detect?

        // TODO: https://developers.openai.com/api/docs/guides/predicted-outputs
        // > When providing a prediction, any tokens provided that are not part
        // > of the final completion are still charged at completion token
        // > rates. See the `rejected_prediction_tokens` property [...]
        // so `completion_tokens_details.accepted_prediction_tokens` are the tokens that are already included,
        // but `completion_tokens_details.rejected_prediction_tokens` are the tokens that still need to be estimated
        // at output token rates

        // TODO: https://developers.openai.com/api/docs/pricing#built-in-tools
    }

    if provider == Provider::Anthropic {
        // prompt caching rates are resolved in `parse_rates`, https://platform.claude.com/docs/en/about-claude/pricing#prompt-caching

        // TODO: https://platform.claude.com/docs/en/about-claude/pricing#long-context-pricing

        // TODO: https://platform.claude.com/docs/en/about-claude/pricing#tool-use-pricing
        //
        // volume discounts: set your own rates in the `[pricing]` table of `conduit.toml`
        // https://platform.claude.com/docs/en/about-claude/pricing#volume-discounts

        // https://platform.claude.com/docs/en/about-claude/pricing#data-residency-pricing
        // > US-only inference via the `inference_geo` parameter incurs a 1.1x multiplier on all token pricing categories
        // > This applies to the 1st party Claude API only. Third-party platforms have their own regional pricing.
        if let Some(inference_geo) = usage.get("inference_geo") && inference_geo.as_str() == Some("us") {
                cost *= 1.1;
        }

        // https://platform.claude.com/docs/en/about-claude/pricing#fast-mode-pricing
        // > 6x standard rates
        // > Fast mode pricing stacks with other pricing modifiers [prompt caching, data residency]
        if let Some(speed) = usage.get("speed") && speed.as_str() == Some("fast") {
                cost *= 6.0;
        }
    }

    // TODO: 3rd party providers https://platform.claude.com/docs/en/about-claude/pricing#third-party-platform-pricing

    Some(cost)
}

// reads deviate on Opus 5.5 and Sonnet 5.5 (0.05x) and Fable 5.1, Fable 5.2, Mythos 5.1
// and Mythos 5.2 (0.025x), so their price entries need an explicit `cached_input`
const CACHED_INPUT_MULTIPLIER: f64 = 0.1;
const CACHE_WRITE_5M_MULTIPLIER: f64 = 1.25;
const CACHE_WRITE_1H_MULTIPLIER: f64 = 2.0;

struct PriceRates {
    input: f64,  // dollars per MTok (million tokens)
    output: f64, // dollars per Mtok
    cached_input: f64,
    cache_write_5m: f64,
    cache_write_1h: f64,
}

fn lookup_rates(custom: Option<&Value>, provider: Provider, model: &str) -> Option<PriceRates> {
    // configured prices win, even over a longer built-in match
    if let Some(rates) = custom.and_then(|table| lookup_in_provider(table, model)) {
        return Some(rates);
    }

    let provider_key = provider.to_string().to_lowercase();
    if let Some(rates) = lookup_in_provider(&FALLBACK_PRICING[&provider_key], model) {
        return Some(rates);
    }

    // if exact provider match didn't contain the model, go through them all
    // to find the first match
    for (_, models) in FALLBACK_PRICING.as_object()? {
        if let Some(rates) = lookup_in_provider(models, model) {
            return Some(rates);
        }
    }

    None
}

fn lookup_in_provider(models: &Value, model: &str) -> Option<PriceRates> {
    let models = models.as_object()?;
    if let Some(rates_json) = models.get(model) {
        return parse_rates(rates_json);
    }

    let mut best_model_match: Option<&str> = None;
    for key in models.keys() {
        if model.starts_with(key.as_str()) {
            match best_model_match {
                Some(prev) if key.len() <= prev.len() => {}
                _ => best_model_match = Some(key),
            }
        }
    }

    parse_rates(models.get(best_model_match?)?)
}

fn parse_rates(v: &Value) -> Option<PriceRates> {
    let input = v.get("input")?.as_f64()?;
    let rate_or = |key: &str, multiplier: f64| {
        v.get(key)
            .and_then(|r| r.as_f64())
            .unwrap_or(input * multiplier)
    };
    Some(PriceRates {
        input,
        output: v.get("output")?.as_f64()?,
        cached_input: rate_or("cached_input", CACHED_INPUT_MULTIPLIER),
        cache_write_5m: rate_or("cache_write_5m", CACHE_WRITE_5M_MULTIPLIER),
        cache_write_1h: rate_or("cache_write_1h", CACHE_WRITE_1H_MULTIPLIER),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_model_match() {
        let usage = json!({"input_tokens": 1_000_000, "output_tokens": 2_000_000});
        let cost = estimate_cost(Provider::OpenAI, "gpt-4.1", &usage).unwrap();
        assert_eq!(cost, 2.0 + (8.0 * 2.0));
    }

    #[test]
    fn prefix_model_match() {
        let usage = json!({"input_tokens": 1_000_000, "output_tokens": 2_000_000});
        let cost = estimate_cost(Provider::OpenAI, "gpt-4.1-2025-04-14", &usage).unwrap();
        assert_eq!(cost, 2.0 + (8.0 * 2.0));
    }

    #[test]
    fn picks_longest_match() {
        let usage = json!({"input_tokens": 1_000_000, "output_tokens": 2_000_000});
        let cost = estimate_cost(Provider::OpenAI, "gpt-5.4", &usage).unwrap(); // also matches "gpt-5"
        assert_eq!(cost, 2.5 + (15.0 * 2.0));
    }

    #[test]
    fn partial_tokens_are_fine() {
        let usage = json!({"input_tokens": 1_000_000});
        let cost = estimate_cost(Provider::OpenAI, "gpt-4.1", &usage).unwrap();
        assert_eq!(cost, 2.0);

        let usage = json!({"output_tokens": 1_000_000});
        let cost = estimate_cost(Provider::OpenAI, "gpt-4.1", &usage).unwrap();
        assert_eq!(cost, 8.0);
    }

    #[test]
    fn scans_all_if_not_found_under_provider() {
        let usage = json!({"input_tokens": 1_000_000, "output_tokens": 2_000_000});

        let cost = estimate_cost(Provider::Unknown, "gpt-4.1", &usage).unwrap();
        assert_eq!(cost, 2.0 + (8.0 * 2.0));

        let cost = estimate_cost(Provider::Anthropic, "gpt-4.1", &usage).unwrap();
        assert_eq!(cost, 2.0 + (8.0 * 2.0));
    }

    #[test]
    fn unknown_model_returns_none() {
        let usage = json!({"input_tokens": 100, "output_tokens": 50});
        assert!(estimate_cost(Provider::OpenAI, "nonexistent-model", &usage).is_none());
    }

    #[test]
    fn missing_required_usage_fields_returns_none() {
        let usage = json!({"something_else": 1234});
        assert!(estimate_cost(Provider::OpenAI, "gpt-4.1", &usage).is_none());
    }

    #[test]
    fn openai_chat_completion_fields() {
        let usage = json!({"prompt_tokens": 1_000_000, "completion_tokens": 2_000_000});
        let cost = estimate_cost(Provider::OpenAI, "gpt-4.1", &usage).unwrap();
        assert_eq!(cost, 2.0 + (8.0 * 2.0));
    }

    #[test]
    fn configured_prefix_prices_unknown_model() {
        let custom = json!({"jev-": {"input": 0.042, "output": 0.0}});
        let usage = json!({"input_tokens": 1_000_000, "output_tokens": 2_000_000});
        let cost = estimate_cost_with(Some(&custom), Provider::Unknown, "jev-1.13.0", &usage);
        assert_eq!(cost, Some(0.042));
    }

    #[test]
    fn configured_price_overrides_built_in() {
        let custom = json!({"gpt-": {"input": 1.0, "output": 1.0}}); // shorter than the built-in "gpt-4.1"
        let usage = json!({"input_tokens": 1_000_000, "output_tokens": 2_000_000});
        let cost = estimate_cost_with(Some(&custom), Provider::OpenAI, "gpt-4.1", &usage);
        assert_eq!(cost, Some(3.0));
    }

    #[test]
    fn unknown_model_without_configured_price_returns_none() {
        let custom = json!({"jev-": {"input": 0.042, "output": 0.0}});
        let usage = json!({"input_tokens": 100, "output_tokens": 50});
        assert!(estimate_cost_with(Some(&custom), Provider::Unknown, "llama-3", &usage).is_none());
    }

    #[test]
    fn anthropic_compounding_multipliers() {
        let model = "claude-opus-4-6";
        let usage = json!({"input_tokens": 1_000_000, "output_tokens": 2_000_000});
        let cost = estimate_cost(Provider::Anthropic, model, &usage).unwrap();
        let baseline = 5.0 + (25.0 * 2.0);
        assert_eq!(cost, baseline);

        let usage =
            json!({"input_tokens": 1_000_000, "output_tokens": 2_000_000, "inference_geo": "us"});
        let cost = estimate_cost(Provider::Anthropic, model, &usage).unwrap();
        assert_eq!(cost, baseline * 1.1);

        let usage = json!({"input_tokens": 1_000_000, "output_tokens": 2_000_000, "inference_geo": "us", "speed": "fast"});
        let cost = estimate_cost(Provider::Anthropic, model, &usage).unwrap();
        assert_eq!(cost, baseline * 1.1 * 6.0);
    }

    #[test]
    fn anthropic_cache_tokens_price_at_cache_rates() {
        let usage = json!({
            "input_tokens": 1_000_000,
            "cache_read_input_tokens": 1_000_000,
            "cache_creation_input_tokens": 1_000_000,
            "output_tokens": 1_000_000,
        });
        let cost = estimate_cost(Provider::Anthropic, "claude-opus-4-6", &usage).unwrap();
        assert_eq!(cost, 5.0 + 0.5 + 6.25 + 25.0);
    }

    #[test]
    fn cache_writes_split_by_ttl() {
        let usage = json!({
            "input_tokens": 0,
            "output_tokens": 0,
            "cache_creation_input_tokens": 2_000_000,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 1_000_000,
                "ephemeral_1h_input_tokens": 1_000_000,
            },
        });
        let cost = estimate_cost(Provider::Anthropic, "claude-opus-4-6", &usage).unwrap();
        assert_eq!(cost, 6.25 + 10.0);

        // simulate a stream where a server tool wrote to the cache after `message_start`
        let usage = json!({
            "input_tokens": 0,
            "output_tokens": 0,
            "cache_creation_input_tokens": 3_000_000,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 1_000_000,
                "ephemeral_1h_input_tokens": 1_000_000,
            },
        });
        let cost = estimate_cost(Provider::Anthropic, "claude-opus-4-6", &usage).unwrap();
        assert_eq!(cost, 12.5 + 10.0);
    }

    #[test]
    fn configured_cache_rates_override_the_multipliers() {
        let custom = json!({
            "jev-": {"input": 4.0, "output": 0.0, "cached_input": 0.2},
            "kuu-": {"input": 4.0, "output": 0.0},
        });
        let usage = json!({"input_tokens": 0, "cache_read_input_tokens": 1_000_000});

        let cost = estimate_cost_with(Some(&custom), Provider::Unknown, "jev-1", &usage);
        assert_eq!(cost, Some(0.2));

        let cost = estimate_cost_with(Some(&custom), Provider::Unknown, "kuu-1", &usage);
        assert_eq!(cost, Some(0.4));
    }
}
