use roder_api::marketplace::{
    MarketplaceDescriptor, MarketplaceKind, MarketplaceSource, MarketplaceState,
};

pub const CLAUDE_DEFAULT_ID: &str = "claude-plugins-official";
pub const CURSOR_DEFAULT_ID: &str = "cursor-plugins";
pub const CODEX_DEFAULT_ID: &str = "codex-plugins";

pub fn default_marketplaces() -> Vec<MarketplaceDescriptor> {
    vec![
        MarketplaceDescriptor {
            id: CLAUDE_DEFAULT_ID.to_string(),
            kind: MarketplaceKind::Claude,
            display_name: "Claude Plugins Official".to_string(),
            source: MarketplaceSource::Github {
                repo: "anthropics/claude-plugins-official".to_string(),
                ref_name: Some("main".to_string()),
                catalog_path: Some(".claude-plugin/marketplace.json".to_string()),
                plugin_root: None,
            },
            homepage: Some("https://github.com/anthropics/claude-plugins-official".to_string()),
            owner_name: Some("Anthropic".to_string()),
            owner_email: Some("support@anthropic.com".to_string()),
            description: Some("Official Claude plugin marketplace".to_string()),
            is_default: true,
            enabled: true,
            state: MarketplaceState::BakedIn,
            last_refreshed_at: None,
            content_hash: None,
        },
        MarketplaceDescriptor {
            id: CURSOR_DEFAULT_ID.to_string(),
            kind: MarketplaceKind::Cursor,
            display_name: "Cursor Marketplace".to_string(),
            source: MarketplaceSource::Github {
                repo: "cursor/plugins".to_string(),
                ref_name: Some("main".to_string()),
                catalog_path: Some(".cursor-plugin/marketplace.json".to_string()),
                plugin_root: None,
            },
            homepage: Some("https://cursor.com/en-US/marketplace".to_string()),
            owner_name: Some("Cursor".to_string()),
            owner_email: Some("plugins@cursor.com".to_string()),
            description: Some("Official Cursor plugin marketplace".to_string()),
            is_default: true,
            enabled: true,
            state: MarketplaceState::BakedIn,
            last_refreshed_at: None,
            content_hash: None,
        },
        MarketplaceDescriptor {
            id: CODEX_DEFAULT_ID.to_string(),
            kind: MarketplaceKind::Codex,
            display_name: "Codex Plugins".to_string(),
            source: MarketplaceSource::Github {
                repo: "openai/plugins".to_string(),
                ref_name: Some("main".to_string()),
                catalog_path: None,
                plugin_root: Some("plugins".to_string()),
            },
            homepage: Some("https://github.com/openai/plugins".to_string()),
            owner_name: Some("OpenAI".to_string()),
            owner_email: None,
            description: Some("Codex plugin examples and curated plugin bundles".to_string()),
            is_default: true,
            enabled: true,
            state: MarketplaceState::BakedIn,
            last_refreshed_at: None,
            content_hash: None,
        },
    ]
}

pub fn default_marketplace(id: &str) -> Option<MarketplaceDescriptor> {
    default_marketplaces()
        .into_iter()
        .find(|marketplace| marketplace.id == id)
}
