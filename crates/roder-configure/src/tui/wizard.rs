use std::collections::{BTreeMap, BTreeSet};

use roder_api::distribution::{DistributionManifest, ExtensionCategory, Profile};

use crate::catalog::Catalog;
use crate::profile::{ProfileExt, ValidationReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    StartingProfile,
    NameVersion,
    InferenceEngines,
    Stores,
    ContextMemoryPolicy,
    ToolProviders,
    UiSurfaces,
    CapabilityReview,
    Output,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildAction {
    GenerateOnly,
    GenerateAndBuild,
    EmitProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WizardState {
    pub step: WizardStep,
    pub starting_profile: Option<Profile>,
    pub manifest: DistributionManifest,
    pub output_dir: Option<String>,
    pub build_action: BuildAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    pub changed_fields: Vec<String>,
    pub required_env: BTreeMap<String, Vec<String>>,
}

impl WizardState {
    pub fn new(starting_profile: Option<Profile>) -> Self {
        let manifest = starting_profile
            .as_ref()
            .map(|profile| profile.manifest.clone())
            .unwrap_or_else(default_manifest);
        Self {
            step: WizardStep::StartingProfile,
            starting_profile,
            manifest,
            output_dir: None,
            build_action: BuildAction::GenerateOnly,
        }
    }

    pub fn next(&mut self) {
        self.step = match self.step {
            WizardStep::StartingProfile => WizardStep::NameVersion,
            WizardStep::NameVersion => WizardStep::InferenceEngines,
            WizardStep::InferenceEngines => WizardStep::Stores,
            WizardStep::Stores => WizardStep::ContextMemoryPolicy,
            WizardStep::ContextMemoryPolicy => WizardStep::ToolProviders,
            WizardStep::ToolProviders => WizardStep::UiSurfaces,
            WizardStep::UiSurfaces => WizardStep::CapabilityReview,
            WizardStep::CapabilityReview => WizardStep::Output,
            WizardStep::Output => WizardStep::Confirm,
            WizardStep::Confirm => WizardStep::Confirm,
        };
    }

    pub fn back(&mut self) {
        self.step = match self.step {
            WizardStep::StartingProfile => WizardStep::StartingProfile,
            WizardStep::NameVersion => WizardStep::StartingProfile,
            WizardStep::InferenceEngines => WizardStep::NameVersion,
            WizardStep::Stores => WizardStep::InferenceEngines,
            WizardStep::ContextMemoryPolicy => WizardStep::Stores,
            WizardStep::ToolProviders => WizardStep::ContextMemoryPolicy,
            WizardStep::UiSurfaces => WizardStep::ToolProviders,
            WizardStep::CapabilityReview => WizardStep::UiSurfaces,
            WizardStep::Output => WizardStep::CapabilityReview,
            WizardStep::Confirm => WizardStep::Output,
        };
    }

    pub fn toggle_extension(&mut self, id: &str) {
        if let Some(index) = self
            .manifest
            .extensions
            .iter()
            .position(|existing| existing == id)
        {
            self.manifest.extensions.remove(index);
        } else {
            self.manifest.extensions.push(id.to_string());
            self.manifest.extensions.sort();
        }
    }

    pub fn capability_review(&self, catalog: &Catalog) -> Result<ValidationReport, String> {
        Profile {
            id: "wizard".to_string(),
            description: "wizard draft".to_string(),
            manifest: self.manifest.clone(),
        }
        .validate(catalog)
        .map_err(|err| err.to_string())
    }

    pub fn confirmation(&self, catalog: &Catalog) -> Result<Confirmation, String> {
        let report = self.capability_review(catalog)?;
        Ok(Confirmation {
            changed_fields: self.changed_fields(),
            required_env: report.required_env,
        })
    }

    fn changed_fields(&self) -> Vec<String> {
        let Some(starting) = &self.starting_profile else {
            return vec!["new-profile".to_string()];
        };
        let mut changed = Vec::new();
        if starting.manifest.name != self.manifest.name {
            changed.push("name".to_string());
        }
        if starting.manifest.version != self.manifest.version {
            changed.push("version".to_string());
        }
        if extension_set(&starting.manifest) != extension_set(&self.manifest) {
            changed.push("extensions".to_string());
        }
        if starting.manifest.include_tui != self.manifest.include_tui
            || starting.manifest.include_app_server != self.manifest.include_app_server
            || starting.manifest.include_cli != self.manifest.include_cli
        {
            changed.push("surfaces".to_string());
        }
        changed
    }
}

pub fn category_ids(catalog: &Catalog, category: ExtensionCategory) -> Vec<String> {
    catalog
        .entries()
        .filter(|entry| entry.entry.category == category)
        .map(|entry| entry.entry.id.clone())
        .collect()
}

fn extension_set(manifest: &DistributionManifest) -> BTreeSet<String> {
    manifest.extensions.iter().cloned().collect()
}

fn default_manifest() -> DistributionManifest {
    DistributionManifest {
        name: "custom-roder".to_string(),
        version: "0.1.0".to_string(),
        include_tui: true,
        include_app_server: true,
        include_cli: true,
        extensions: Vec::new(),
        default_provider: None,
        default_session_store: None,
        config_overrides: serde_json::Value::Null,
    }
}
