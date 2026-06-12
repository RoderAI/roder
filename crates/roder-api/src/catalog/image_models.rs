//! Built-in image generation provider/model catalog.
//!
//! Image generation models are deliberately separate from the chat model
//! catalog so they never appear in chat model pickers; media-provider code
//! queries them through [`built_in_image_providers`] and
//! [`image_models_for_provider`].

use crate::media::ImageModelDescriptor;

pub const IMAGE_PROVIDER_OPENAI: &str = "openai";
pub const IMAGE_PROVIDER_GOOGLE: &str = "google";

#[derive(Debug, Clone, Copy)]
pub struct ImageProviderCatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub default_model: &'static str,
    pub base_url: &'static str,
    pub env_key: &'static str,
    pub env_aliases: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct ImageModelCatalogEntry {
    pub id: &'static str,
    pub display_name: &'static str,
    pub provider: &'static str,
    pub is_default: bool,
    pub legacy: bool,
    pub supports_edit: bool,
    pub supports_multiple_outputs: bool,
    pub supported_aspect_ratios: &'static [&'static str],
    pub supported_sizes: &'static [&'static str],
    pub supported_image_sizes: &'static [&'static str],
    pub supports_transparent_background: bool,
    pub supports_partial_images: bool,
}

impl ImageModelCatalogEntry {
    pub fn descriptor(&self) -> ImageModelDescriptor {
        ImageModelDescriptor {
            id: self.id.to_string(),
            display_name: self.display_name.to_string(),
            provider: self.provider.to_string(),
            is_default: self.is_default,
            legacy: self.legacy,
            supports_edit: self.supports_edit,
            supports_multiple_outputs: self.supports_multiple_outputs,
            supported_aspect_ratios: to_strings(self.supported_aspect_ratios),
            supported_sizes: to_strings(self.supported_sizes),
            supported_image_sizes: to_strings(self.supported_image_sizes),
            supports_transparent_background: self.supports_transparent_background,
            supports_partial_images: self.supports_partial_images,
        }
    }
}

fn to_strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

const IMAGE_PROVIDERS: &[ImageProviderCatalogEntry] = &[
    ImageProviderCatalogEntry {
        id: IMAGE_PROVIDER_OPENAI,
        name: "OpenAI GPT Image",
        default_model: "gpt-image-2",
        base_url: "https://api.openai.com/v1",
        env_key: "OPENAI_API_KEY",
        env_aliases: &[],
    },
    ImageProviderCatalogEntry {
        id: IMAGE_PROVIDER_GOOGLE,
        name: "Google Gemini Images",
        default_model: "gemini-3.1-flash-image",
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        env_key: "GEMINI_API_KEY",
        env_aliases: &["GEMINI_API_TOKEN", "GOOGLE_API_KEY"],
    },
];

const OPENAI_SIZES: &[&str] = &["auto", "1024x1024", "1536x1024", "1024x1536"];

/// All Nano Banana models share the same documented aspect ratio set.
const GOOGLE_ASPECT_RATIOS: &[&str] = &[
    "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9",
];

const GOOGLE_IMAGE_SIZES: &[&str] = &["1K", "2K", "4K"];

const IMAGE_MODELS: &[ImageModelCatalogEntry] = &[
    ImageModelCatalogEntry {
        id: "gpt-image-2",
        display_name: "GPT Image 2",
        provider: IMAGE_PROVIDER_OPENAI,
        is_default: true,
        legacy: false,
        supports_edit: true,
        supports_multiple_outputs: true,
        supported_aspect_ratios: &[],
        supported_sizes: OPENAI_SIZES,
        supported_image_sizes: &[],
        supports_transparent_background: true,
        supports_partial_images: false,
    },
    ImageModelCatalogEntry {
        id: "gpt-image-1.5",
        display_name: "GPT Image 1.5",
        provider: IMAGE_PROVIDER_OPENAI,
        is_default: false,
        legacy: true,
        supports_edit: true,
        supports_multiple_outputs: true,
        supported_aspect_ratios: &[],
        supported_sizes: OPENAI_SIZES,
        supported_image_sizes: &[],
        supports_transparent_background: true,
        supports_partial_images: false,
    },
    ImageModelCatalogEntry {
        id: "gpt-image-1",
        display_name: "GPT Image 1",
        provider: IMAGE_PROVIDER_OPENAI,
        is_default: false,
        legacy: true,
        supports_edit: true,
        supports_multiple_outputs: true,
        supported_aspect_ratios: &[],
        supported_sizes: OPENAI_SIZES,
        supported_image_sizes: &[],
        supports_transparent_background: true,
        supports_partial_images: false,
    },
    ImageModelCatalogEntry {
        id: "gpt-image-1-mini",
        display_name: "GPT Image 1 Mini",
        provider: IMAGE_PROVIDER_OPENAI,
        is_default: false,
        legacy: true,
        supports_edit: true,
        supports_multiple_outputs: true,
        supported_aspect_ratios: &[],
        supported_sizes: OPENAI_SIZES,
        supported_image_sizes: &[],
        supports_transparent_background: true,
        supports_partial_images: false,
    },
    ImageModelCatalogEntry {
        id: "gemini-3.1-flash-image",
        display_name: "Nano Banana 2",
        provider: IMAGE_PROVIDER_GOOGLE,
        is_default: true,
        legacy: false,
        supports_edit: true,
        supports_multiple_outputs: false,
        supported_aspect_ratios: GOOGLE_ASPECT_RATIOS,
        supported_sizes: &[],
        supported_image_sizes: GOOGLE_IMAGE_SIZES,
        supports_transparent_background: false,
        supports_partial_images: false,
    },
    ImageModelCatalogEntry {
        id: "gemini-3-pro-image",
        display_name: "Nano Banana Pro",
        provider: IMAGE_PROVIDER_GOOGLE,
        is_default: false,
        legacy: false,
        supports_edit: true,
        supports_multiple_outputs: false,
        supported_aspect_ratios: GOOGLE_ASPECT_RATIOS,
        supported_sizes: &[],
        supported_image_sizes: GOOGLE_IMAGE_SIZES,
        supports_transparent_background: false,
        supports_partial_images: false,
    },
    ImageModelCatalogEntry {
        id: "gemini-2.5-flash-image",
        display_name: "Nano Banana",
        provider: IMAGE_PROVIDER_GOOGLE,
        is_default: false,
        legacy: false,
        supports_edit: true,
        supports_multiple_outputs: false,
        supported_aspect_ratios: GOOGLE_ASPECT_RATIOS,
        supported_sizes: &[],
        supported_image_sizes: &[],
        supports_transparent_background: false,
        supports_partial_images: false,
    },
];

pub fn built_in_image_providers() -> &'static [ImageProviderCatalogEntry] {
    IMAGE_PROVIDERS
}

pub fn lookup_image_provider(id: &str) -> Option<&'static ImageProviderCatalogEntry> {
    IMAGE_PROVIDERS.iter().find(|provider| provider.id == id)
}

pub fn image_models_for_provider(provider: &str) -> Vec<&'static ImageModelCatalogEntry> {
    IMAGE_MODELS
        .iter()
        .filter(|model| model.provider == provider)
        .collect()
}

pub fn lookup_image_model(provider: &str, id: &str) -> Option<&'static ImageModelCatalogEntry> {
    IMAGE_MODELS
        .iter()
        .find(|model| model.provider == provider && model.id == id)
}

pub fn image_model_descriptors(provider: &str) -> Vec<ImageModelDescriptor> {
    image_models_for_provider(provider)
        .into_iter()
        .map(ImageModelCatalogEntry::descriptor)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_generation_catalog_lists_openai_and_google_models() {
        let openai_ids: Vec<&str> = image_models_for_provider(IMAGE_PROVIDER_OPENAI)
            .iter()
            .map(|model| model.id)
            .collect();
        assert_eq!(
            openai_ids,
            vec![
                "gpt-image-2",
                "gpt-image-1.5",
                "gpt-image-1",
                "gpt-image-1-mini"
            ]
        );

        let google_ids: Vec<&str> = image_models_for_provider(IMAGE_PROVIDER_GOOGLE)
            .iter()
            .map(|model| model.id)
            .collect();
        assert_eq!(
            google_ids,
            vec![
                "gemini-3.1-flash-image",
                "gemini-3-pro-image",
                "gemini-2.5-flash-image"
            ]
        );
    }

    #[test]
    fn image_generation_default_models_match_provider_entries() {
        for provider in built_in_image_providers() {
            let default = image_models_for_provider(provider.id)
                .into_iter()
                .find(|model| model.is_default)
                .expect("image provider declares a default model");
            assert_eq!(default.id, provider.default_model);
        }
    }

    #[test]
    fn non_primary_openai_image_models_are_marked_legacy() {
        for model in image_models_for_provider(IMAGE_PROVIDER_OPENAI) {
            assert_eq!(model.legacy, model.id != "gpt-image-2", "{}", model.id);
        }
    }

    #[test]
    fn google_image_size_support_is_model_specific() {
        let nano_banana = lookup_image_model(IMAGE_PROVIDER_GOOGLE, "gemini-2.5-flash-image")
            .expect("nano banana entry");
        assert!(nano_banana.supported_image_sizes.is_empty());
        assert!(!nano_banana.supported_aspect_ratios.is_empty());

        for id in ["gemini-3.1-flash-image", "gemini-3-pro-image"] {
            let model = lookup_image_model(IMAGE_PROVIDER_GOOGLE, id).expect("model entry");
            assert_eq!(model.supported_image_sizes, GOOGLE_IMAGE_SIZES, "{id}");
        }
    }

    #[test]
    fn image_models_use_display_aliases_for_nano_banana() {
        assert_eq!(
            lookup_image_model(IMAGE_PROVIDER_GOOGLE, "gemini-3.1-flash-image")
                .unwrap()
                .display_name,
            "Nano Banana 2"
        );
        assert_eq!(
            lookup_image_model(IMAGE_PROVIDER_GOOGLE, "gemini-3-pro-image")
                .unwrap()
                .display_name,
            "Nano Banana Pro"
        );
        assert_eq!(
            lookup_image_model(IMAGE_PROVIDER_GOOGLE, "gemini-2.5-flash-image")
                .unwrap()
                .display_name,
            "Nano Banana"
        );
    }

    #[test]
    fn image_model_descriptor_conversion_keeps_capability_metadata() {
        let descriptor = lookup_image_model(IMAGE_PROVIDER_OPENAI, "gpt-image-2")
            .unwrap()
            .descriptor();
        assert!(descriptor.is_default);
        assert!(descriptor.supports_edit);
        assert!(descriptor.supports_transparent_background);
        assert!(
            descriptor
                .supported_sizes
                .contains(&"1536x1024".to_string())
        );
    }

    #[test]
    fn image_generation_models_are_absent_from_chat_model_catalog() {
        for model in IMAGE_MODELS {
            assert!(
                crate::catalog::lookup_model(model.id).is_none(),
                "image model {} must not appear in the chat model catalog",
                model.id
            );
        }
    }
}
