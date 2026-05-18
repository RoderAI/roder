pub mod bootstrap;
pub mod cache;
pub mod defaults;
pub mod fetch;
pub mod source;
pub mod store;

pub use bootstrap::install_default_marketplaces;
pub use defaults::{CLAUDE_DEFAULT_ID, CODEX_DEFAULT_ID, CURSOR_DEFAULT_ID, default_marketplaces};
pub use fetch::{RawMarketplaceCatalog, read_catalog_from_root, refresh_marketplace};
pub use source::{
    infer_kind_from_root, infer_kind_from_source, resolve_marketplace_source, resolve_source,
};
pub use store::{
    MarketplaceStore, load_marketplace_store, marketplace_store_path, save_marketplace_store,
};

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::marketplace::{DefaultMarketplaceSelection, MarketplaceState};

    #[test]
    fn store_loads_baked_in_defaults_when_empty() {
        let store = MarketplaceStore::default().with_baked_in_defaults();

        assert_eq!(store.marketplaces.len(), 3);
        assert!(
            store
                .marketplaces
                .iter()
                .all(|marketplace| marketplace.is_default)
        );
    }

    #[test]
    fn default_selection_ids_are_stable() {
        assert_eq!(
            DefaultMarketplaceSelection::All.selected_ids(),
            &[CLAUDE_DEFAULT_ID, CURSOR_DEFAULT_ID, CODEX_DEFAULT_ID]
        );
    }

    #[test]
    fn upsert_marketplace_is_idempotent() {
        let mut store = MarketplaceStore::default().with_baked_in_defaults();
        let mut marketplace = store.marketplaces[0].clone();
        marketplace.state = MarketplaceState::Installed;
        let id = marketplace.id.clone();

        store.upsert_marketplace(marketplace.clone());
        store.upsert_marketplace(marketplace);

        assert_eq!(
            store
                .marketplaces
                .iter()
                .filter(|marketplace| marketplace.id == id)
                .count(),
            1
        );
    }
}
