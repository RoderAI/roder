use std::collections::BTreeMap;

use roder_api::marketplace::{
    DedupedMarketplacePlugin, MarketplaceKind, MarketplacePluginEntry, MarketplacePluginVariant,
    PluginIdentityKey, variant_key,
};

pub fn dedupe_plugins(
    entries: &[MarketplacePluginEntry],
    installed_variants: &[String],
) -> Vec<DedupedMarketplacePlugin> {
    let mut groups = BTreeMap::<String, Vec<&MarketplacePluginEntry>>::new();
    for entry in entries {
        groups
            .entry(group_key(&entry.identity_key, &entry.kind))
            .or_default()
            .push(entry);
    }
    let mut plugins = groups
        .into_values()
        .map(|mut entries| {
            entries
                .sort_by(|left, right| variant_order(&left.kind).cmp(&variant_order(&right.kind)));
            let first = entries[0];
            let variants = entries
                .iter()
                .map(|entry| variant_from_entry(entry))
                .collect::<Vec<_>>();
            let recommended_variant_key = variants
                .first()
                .map(|variant| variant_key(&variant.marketplace_id, &variant.plugin_id));
            DedupedMarketplacePlugin {
                identity_key: first.identity_key.clone(),
                display_name: first.display_name.clone(),
                description: first.description.clone(),
                related_candidates: Vec::new(),
                recommended_variant_key,
                installed_variants: variants
                    .iter()
                    .map(|variant| variant_key(&variant.marketplace_id, &variant.plugin_id))
                    .filter(|key| installed_variants.contains(key))
                    .collect(),
                variants,
            }
        })
        .collect::<Vec<_>>();
    attach_related_candidates(&mut plugins);
    plugins.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    plugins
}

fn variant_from_entry(entry: &MarketplacePluginEntry) -> MarketplacePluginVariant {
    MarketplacePluginVariant {
        marketplace_id: entry.marketplace_id.clone(),
        plugin_id: entry.plugin_id.clone(),
        kind: entry.kind.clone(),
        source: entry.source.clone(),
        component_hints: entry.component_hints.clone(),
        capability_hints: entry.capability_hints.clone(),
        version: entry.version.clone(),
        content_hash: None,
        risk: entry.risk.clone(),
    }
}

fn attach_related_candidates(plugins: &mut [DedupedMarketplacePlugin]) {
    for index in 0..plugins.len() {
        let related = plugins
            .iter()
            .enumerate()
            .filter(|(candidate_index, candidate)| {
                *candidate_index != index
                    && candidate.identity_key.normalized_name
                        == plugins[index].identity_key.normalized_name
            })
            .flat_map(|(_, candidate)| candidate.variants.clone())
            .collect::<Vec<_>>();
        plugins[index].related_candidates = related;
    }
}

fn group_key(identity: &PluginIdentityKey, kind: &MarketplaceKind) -> String {
    if let Some(repository) = &identity.repository {
        format!("repo:{repository}")
    } else if let Some(domain) = &identity.homepage_domain {
        format!("home:{domain}:{}", identity.normalized_name)
    } else {
        format!("kind:{kind:?}:{}", identity.canonical_slug)
    }
}

fn variant_order(kind: &MarketplaceKind) -> u8 {
    match kind {
        MarketplaceKind::Claude => 0,
        MarketplaceKind::Codex => 1,
        MarketplaceKind::Cursor => 2,
        MarketplaceKind::Roder => 3,
        MarketplaceKind::Custom => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::marketplace::{
        MarketplacePluginRisk, PluginComponentHints, PluginSource, normalize_slug,
    };

    #[test]
    fn groups_same_repository_across_marketplaces() {
        let entries = vec![
            entry("claude-plugins-official", MarketplaceKind::Claude),
            entry("codex-plugins", MarketplaceKind::Codex),
            entry("cursor-plugins", MarketplaceKind::Cursor),
        ];

        let grouped = dedupe_plugins(&entries, &[]);

        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].variants.len(), 3);
        assert_eq!(
            grouped[0].recommended_variant_key.as_deref(),
            Some("claude-plugins-official:superpowers")
        );
    }

    #[test]
    fn groups_pairwise_default_provider_combinations() {
        for pair in [
            (MarketplaceKind::Claude, MarketplaceKind::Codex),
            (MarketplaceKind::Cursor, MarketplaceKind::Codex),
            (MarketplaceKind::Claude, MarketplaceKind::Cursor),
        ] {
            let entries = vec![
                entry("left-marketplace", pair.0),
                entry("right-marketplace", pair.1),
            ];

            let grouped = dedupe_plugins(&entries, &[]);

            assert_eq!(grouped.len(), 1);
            assert_eq!(grouped[0].variants.len(), 2);
        }
    }

    #[test]
    fn keeps_weak_name_only_matches_separate() {
        let mut left = entry("claude-plugins-official", MarketplaceKind::Claude);
        let mut right = entry("cursor-plugins", MarketplaceKind::Cursor);
        left.identity_key.repository = None;
        left.identity_key.homepage_domain = None;
        right.identity_key.repository = None;
        right.identity_key.homepage_domain = None;

        let grouped = dedupe_plugins(&[left, right], &[]);

        assert_eq!(grouped.len(), 2);
        assert!(grouped.iter().all(|plugin| plugin.variants.len() == 1));
        assert!(
            grouped
                .iter()
                .all(|plugin| plugin.related_candidates.len() == 1)
        );
    }

    #[test]
    fn groups_custom_marketplace_variants_by_strong_identity() {
        let entries = vec![
            entry("codex-plugins", MarketplaceKind::Codex),
            entry("team-marketplace", MarketplaceKind::Custom),
        ];

        let grouped = dedupe_plugins(&entries, &[]);

        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].variants.len(), 2);
    }

    fn entry(marketplace_id: &str, kind: MarketplaceKind) -> MarketplacePluginEntry {
        MarketplacePluginEntry {
            marketplace_id: marketplace_id.to_string(),
            plugin_id: "superpowers".to_string(),
            identity_key: PluginIdentityKey {
                canonical_slug: normalize_slug("github.com/obra/superpowers"),
                normalized_name: "superpowers".to_string(),
                repository: Some("https://github.com/obra/superpowers".to_string()),
                homepage_domain: Some("github.com".to_string()),
                author_name: Some("Jesse Vincent".to_string()),
            },
            display_name: "Superpowers".to_string(),
            description: Some("Workflow skills".to_string()),
            kind,
            version: Some("1.0.0".to_string()),
            source: PluginSource::MarketplacePath {
                marketplace_id: marketplace_id.to_string(),
                path: "superpowers".to_string(),
            },
            homepage: Some("https://github.com/obra/superpowers".to_string()),
            repository: Some("https://github.com/obra/superpowers".to_string()),
            author_name: Some("Jesse Vincent".to_string()),
            category: None,
            tags: Vec::new(),
            component_hints: PluginComponentHints {
                skills: true,
                ..PluginComponentHints::default()
            },
            capability_hints: Vec::new(),
            risk: MarketplacePluginRisk::Passive,
            raw_manifest: serde_json::json!({ "name": "superpowers" }),
        }
    }
}
