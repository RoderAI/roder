use roder_api::marketplace::{
    DefaultMarketplaceSelection, MarketplaceDescriptor, MarketplaceState, validate_marketplace_id,
};

use super::defaults::default_marketplace;
use super::store::{load_marketplace_store, save_marketplace_store};

pub fn install_default_marketplaces(
    selection: DefaultMarketplaceSelection,
) -> anyhow::Result<Vec<MarketplaceDescriptor>> {
    let mut store = load_marketplace_store()?;
    let mut installed = Vec::new();
    for id in selection.selected_ids() {
        validate_marketplace_id(id)?;
        let mut marketplace = default_marketplace(id)
            .ok_or_else(|| anyhow::anyhow!("unknown default marketplace {id}"))?;
        marketplace.state = MarketplaceState::Installed;
        marketplace.enabled = true;
        store.upsert_marketplace(marketplace.clone());
        installed.push(marketplace);
    }
    save_marketplace_store(&store)?;
    Ok(installed)
}
