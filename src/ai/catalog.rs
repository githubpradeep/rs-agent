//! Built-in model catalog (pi-style), extracted from the reference pi
//! `*.models.ts` files. Regenerate with `scripts/sync-model-catalog.py`.

use serde::Deserialize;
use std::sync::OnceLock;

const CATALOG_JSON: &str = include_str!("../../data/models.catalog.json");

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub api: String,
    pub base_url: String,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub context_window: u64,
    #[serde(default)]
    pub max_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    models: Vec<CatalogModel>,
}

fn catalog() -> &'static [CatalogModel] {
    static CAT: OnceLock<Vec<CatalogModel>> = OnceLock::new();
    CAT.get_or_init(|| {
        serde_json::from_str::<CatalogFile>(CATALOG_JSON)
            .map(|f| f.models)
            .unwrap_or_default()
    })
}

/// Every model in the built-in catalog.
pub fn all_models() -> &'static [CatalogModel] {
    catalog()
}

/// Distinct provider ids present in the catalog (sorted).
pub fn catalog_providers() -> Vec<&'static str> {
    let mut v: Vec<&str> = catalog()
        .iter()
        .map(|m| m.provider.as_str())
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

pub fn models_for_provider(provider: &str) -> Vec<&'static CatalogModel> {
    catalog()
        .iter()
        .filter(|m| m.provider.eq_ignore_ascii_case(provider))
        .collect()
}

pub fn find_model(provider: &str, model_id: &str) -> Option<&'static CatalogModel> {
    catalog().iter().find(|m| {
        m.provider.eq_ignore_ascii_case(provider) && (m.id == model_id || m.name == model_id)
    })
}

/// `provider/id` display strings for one provider.
pub fn displays_for_provider(provider: &str) -> Vec<String> {
    models_for_provider(provider)
        .into_iter()
        .map(|m| format!("{}/{}", m.provider, m.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_nonempty() {
        assert!(all_models().len() > 500, "expected large pi-style catalog");
        assert!(catalog_providers().len() > 20);
    }

    #[test]
    fn openrouter_has_many_models() {
        assert!(models_for_provider("openrouter").len() > 100);
    }

    #[test]
    fn anthropic_models_present() {
        assert!(models_for_provider("anthropic").len() > 5);
    }
}
