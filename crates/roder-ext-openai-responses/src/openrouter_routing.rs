use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// OpenRouter endpoint selection for a request: provider routing preferences
/// sent verbatim as the body's `provider` object (`only`, `order`, `ignore`,
/// `allow_fallbacks`, `sort`, `max_price`, `quantizations`, ...) and the
/// top-level `service_tier`. See
/// <https://openrouter.ai/docs/guides/routing/provider-selection> and
/// <https://openrouter.ai/docs/guides/features/service-tiers>.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterRouting {
    /// Applied to every model without a `models` entry. An empty object sends
    /// no `provider` field, leaving OpenRouter's default load balancing.
    pub default: Map<String, Value>,
    /// Keyed by OpenRouter model slug (`deepseek/deepseek-v4.1-flash`); a
    /// model entry replaces `default` rather than merging into it.
    pub models: HashMap<String, Map<String, Value>>,
    /// `default`, `flex`, `priority`, or `ultrafast`. A per-request runtime
    /// service tier takes precedence.
    pub service_tier: Option<String>,
}

impl Default for OpenRouterRouting {
    /// Roder routes OpenRouter requests to the cheapest endpoint first.
    fn default() -> Self {
        Self::from_config(None, HashMap::new(), None)
    }
}

impl OpenRouterRouting {
    /// `default: None` means unconfigured and selects price-first routing.
    pub fn from_config(
        default: Option<Map<String, Value>>,
        models: HashMap<String, Map<String, Value>>,
        service_tier: Option<String>,
    ) -> Self {
        Self {
            default: default.unwrap_or_else(price_first),
            models,
            service_tier: service_tier
                .map(|tier| tier.trim().to_string())
                .filter(|tier| !tier.is_empty()),
        }
    }

    pub fn for_model(&self, model: &str) -> Option<&Map<String, Value>> {
        Some(self.models.get(model).unwrap_or(&self.default)).filter(|routing| !routing.is_empty())
    }

    pub(crate) fn apply(&self, body: &mut Value) {
        if let Some(routing) = body
            .get("model")
            .and_then(Value::as_str)
            .and_then(|model| self.for_model(model))
        {
            body["provider"] = Value::Object(routing.clone());
        }
        if body.get("service_tier").is_none()
            && let Some(service_tier) = &self.service_tier
        {
            body["service_tier"] = json!(service_tier);
        }
    }
}

fn price_first() -> Map<String, Value> {
    Map::from_iter([("sort".to_string(), json!("price"))])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn unconfigured_routing_sorts_by_price() {
        let mut body = json!({ "model": "x-ai/grok-4.6" });
        OpenRouterRouting::default().apply(&mut body);
        assert_eq!(body["provider"], json!({ "sort": "price" }));
        assert!(body.get("service_tier").is_none());
    }

    #[test]
    fn model_routing_replaces_default_routing() {
        let routing = OpenRouterRouting::from_config(
            Some(object(json!({ "sort": "throughput" }))),
            HashMap::from([(
                "deepseek/deepseek-v4.1-flash".to_string(),
                object(json!({ "only": ["deepinfra/fp8"], "allow_fallbacks": false })),
            )]),
            None,
        );

        let mut pinned = json!({ "model": "deepseek/deepseek-v4.1-flash" });
        routing.apply(&mut pinned);
        assert_eq!(
            pinned["provider"],
            json!({ "only": ["deepinfra/fp8"], "allow_fallbacks": false })
        );

        let mut other = json!({ "model": "x-ai/grok-4.6" });
        routing.apply(&mut other);
        assert_eq!(other["provider"], json!({ "sort": "throughput" }));
    }

    #[test]
    fn empty_routing_restores_openrouter_load_balancing() {
        let routing = OpenRouterRouting::from_config(Some(Map::new()), HashMap::new(), None);
        let mut body = json!({ "model": "x-ai/grok-4.6" });
        routing.apply(&mut body);
        assert!(body.get("provider").is_none());
    }

    #[test]
    fn empty_model_routing_load_balances_that_model_only() {
        let routing = OpenRouterRouting::from_config(
            None,
            HashMap::from([("x-ai/grok-4.6".to_string(), Map::new())]),
            None,
        );

        let mut balanced = json!({ "model": "x-ai/grok-4.6" });
        routing.apply(&mut balanced);
        assert!(balanced.get("provider").is_none());

        let mut other = json!({ "model": "moonshotai/kimi-k3" });
        routing.apply(&mut other);
        assert_eq!(other["provider"], json!({ "sort": "price" }));
    }

    #[test]
    fn configured_service_tier_yields_to_request_tier() {
        let routing = OpenRouterRouting::from_config(None, HashMap::new(), Some("flex".into()));

        let mut body = json!({ "model": "openai/gpt-5.5" });
        routing.apply(&mut body);
        assert_eq!(body["service_tier"], "flex");

        let mut fast = json!({ "model": "openai/gpt-5.5", "service_tier": "priority" });
        routing.apply(&mut fast);
        assert_eq!(fast["service_tier"], "priority");
    }
}
