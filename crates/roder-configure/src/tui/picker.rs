use roder_api::distribution::ExtensionCategory;

use crate::catalog::CatalogEntry;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerState {
    pub query: String,
    pub category: Option<ExtensionCategory>,
    pub selected_index: usize,
}

impl PickerState {
    pub fn matches<'a>(&self, entries: &'a [&'a CatalogEntry]) -> Vec<&'a CatalogEntry> {
        let query = self.query.to_ascii_lowercase();
        entries
            .iter()
            .copied()
            .filter(|entry| {
                self.category
                    .as_ref()
                    .is_none_or(|category| &entry.entry.category == category)
            })
            .filter(|entry| {
                query.is_empty()
                    || entry.entry.id.contains(&query)
                    || entry
                        .entry
                        .display_name
                        .to_ascii_lowercase()
                        .contains(&query)
            })
            .collect()
    }

    pub fn help(entry: &CatalogEntry) -> String {
        match entry
            .entry
            .docs_url
            .as_deref()
            .filter(|url| !url.is_empty())
        {
            Some(url) => format!("{} - {url}", entry.entry.description),
            None => entry.entry.description.clone(),
        }
    }
}
