//! Provider/model registry — mid-session switching + static catalog.
//!
//! Active choice is a `(provider, model)` pair. The built-in catalog lists
//! curated models per provider; the picker shows catalog entries for every
//! provider that has configured auth and a runnable API client.

use crate::ai::anthropic::AnthropicProvider;
use crate::ai::catalog::{self, CatalogModel};
use crate::ai::openai::OpenAIProvider;
use crate::ai::provider::Provider;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
}

impl ModelRef {
    pub fn display(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }

    /// Parse `provider/model`, `provider:model`, or bare `model` (uses `default_provider`).
    pub fn parse(input: &str, default_provider: &str) -> Self {
        let s = input.trim();
        if let Some((p, m)) = s.split_once('/') {
            if !p.is_empty() && !m.is_empty() && is_known_provider(p) {
                return Self {
                    provider: p.to_lowercase(),
                    model: m.to_string(),
                };
            }
        }
        if let Some((p, m)) = s.split_once(':') {
            if !p.is_empty() && !m.is_empty() && is_known_provider(p) {
                return Self {
                    provider: p.to_lowercase(),
                    model: m.to_string(),
                };
            }
        }
        Self {
            provider: default_provider.to_lowercase(),
            model: s.to_string(),
        }
    }
}

/// All provider ids we know about (from the catalog), sorted.
pub fn known_providers() -> Vec<String> {
    let mut v: Vec<String> = catalog::catalog_providers()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn is_known_provider(name: &str) -> bool {
    let n = name.to_lowercase();
    catalog::catalog_providers()
        .iter()
        .any(|p| p.eq_ignore_ascii_case(&n))
}

fn canonicalize_provider(name: &str) -> String {
    name.to_lowercase()
}

pub fn default_model_for(provider: &str) -> String {
    let p = canonicalize_provider(provider);
    if let Some(m) = catalog::models_for_provider(&p).first() {
        return m.id.clone();
    }
    "gpt-4o".to_string()
}

pub fn api_key_env_for(provider: &str) -> &'static str {
    match canonicalize_provider(provider).as_str() {
        "openai" => "OPENAI_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "groq" => "GROQ_API_KEY",
        "deepseek" => "DEEPSEEK_API_KEY",
        "mistral" => "MISTRAL_API_KEY",
        _ => "API_KEY",
    }
}

/// True when we can construct a runnable client for this provider's catalog API.
pub fn supports_runtime(provider: &str) -> bool {
    let p = canonicalize_provider(provider);
    let models = catalog::models_for_provider(&p);
    let Some(sample) = models.first() else {
        return false;
    };
    matches!(
        sample.api.as_str(),
        "openai-completions" | "openai-responses" | "anthropic-messages" | "mistral-conversations"
    )
}

pub fn has_configured_auth(provider: &str) -> bool {
    let p = canonicalize_provider(provider);
    let env = api_key_env_for(&p);
    if std::env::var(env).is_ok() {
        return true;
    }
    crate::config::Secrets::load().get_key(&p).is_some()
}

/// Console / signup URL where the user can create an API key for `provider`.
pub fn provider_connect_url(provider: &str) -> Option<&'static str> {
    match canonicalize_provider(provider).as_str() {
        "anthropic" => Some("https://console.anthropic.com/settings/keys"),
        "openai" => Some("https://platform.openai.com/api-keys"),
        "openrouter" => Some("https://openrouter.ai/keys"),
        "groq" => Some("https://console.groq.com/keys"),
        "deepseek" => Some("https://platform.deepseek.com/api_keys"),
        "mistral" => Some("https://console.mistral.ai/api-keys/"),
        _ => None,
    }
}

/// Best-effort open the connect URL in the user's browser.
pub fn open_provider_connect_url(provider: &str) -> Result<String, String> {
    let url = provider_connect_url(provider).ok_or_else(|| {
        format!(
            "No known signup URL for `{}`. Set {} manually.",
            provider,
            api_key_env_for(provider)
        )
    })?;
    open_url(url)?;
    Ok(url.to_string())
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let status = std::process::Command::new("xdg-open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .status();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
        return Err("cannot open URLs on this OS".into());
    }
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("browser open failed with {}", s)),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Runnable providers for the `/provider` picker, with status labels.
pub fn provider_picker_rows() -> Vec<String> {
    let mut rows = Vec::new();
    for p in known_providers() {
        if !supports_runtime(&p) {
            continue;
        }
        let ready = has_configured_auth(&p);
        let mark = if ready { "ready" } else { "needs key" };
        let n = catalog::models_for_provider(&canonicalize_provider(&p)).len();
        let url = provider_connect_url(&p).unwrap_or("");
        if ready {
            rows.push(format!("{p}  [{mark}]  {n} models"));
        } else if url.is_empty() {
            rows.push(format!("{p}  [{mark}]  set {}", api_key_env_for(&p)));
        } else {
            rows.push(format!("{p}  [{mark}]  {url}"));
        }
    }
    // Ensure core ones always appear even if catalog empty
    for p in ["anthropic", "openai", "openrouter"] {
        if !rows.iter().any(|r| r.starts_with(&format!("{p} "))) && supports_runtime(p) {
            let ready = has_configured_auth(p);
            let mark = if ready { "ready" } else { "needs key" };
            let url = provider_connect_url(p).unwrap_or("");
            rows.push(format!("{p}  [{mark}]  {url}"));
        }
    }
    rows.sort();
    rows.dedup();
    rows
}

/// Extract provider id from a picker row (`"openrouter  [needs key]  https://..."`).
pub fn provider_from_picker_row(row: &str) -> String {
    row.split_whitespace().next().unwrap_or("").to_lowercase()
}

#[derive(Debug, Clone, Default)]
pub struct CreateProviderOpts {
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub timeout_secs: u64,
}

impl CreateProviderOpts {
    pub fn new() -> Self {
        Self {
            timeout_secs: 300,
            ..Default::default()
        }
    }
}

fn catalog_entry_for(provider: &str, model: Option<&str>) -> Option<&'static CatalogModel> {
    if let Some(m) = model.filter(|s| !s.is_empty()) {
        if let Some(found) = catalog::find_model(provider, m) {
            return Some(found);
        }
    }
    catalog::models_for_provider(provider).first().copied()
}

fn normalize_openai_base(url: &str) -> String {
    let u = url.trim_end_matches('/').to_string();
    if u.contains("/v1") || u.ends_with("/openai") {
        u
    } else {
        format!("{}/v1", u)
    }
}

fn normalize_anthropic_base(url: &str) -> String {
    let u = url.trim_end_matches('/').to_string();
    if u.ends_with("/v1") {
        u
    } else {
        format!("{}/v1", u)
    }
}

/// `(api, normalized_base_url)` for a catalog model — used to decide mid-session
/// client recreation when switching models under the same provider id (e.g.
/// OpenCode Zen OpenAI vs Anthropic endpoints).
fn client_fingerprint(provider: &str, model: &str) -> Option<(String, String)> {
    let p = canonicalize_provider(provider);
    let m = catalog::find_model(&p, model)?;
    let base = match m.api.as_str() {
        "anthropic-messages" => normalize_anthropic_base(&m.base_url),
        _ => normalize_openai_base(&m.base_url),
    };
    Some((m.api.clone(), base))
}

/// True when switching models requires a new HTTP client (different API or base URL).
pub fn provider_client_needs_recreate(
    old_provider: &str,
    old_model: &str,
    new_provider: &str,
    new_model: &str,
) -> bool {
    if !old_provider.eq_ignore_ascii_case(new_provider) {
        return true;
    }
    match (
        client_fingerprint(old_provider, old_model),
        client_fingerprint(new_provider, new_model),
    ) {
        (Some(a), Some(b)) => a != b,
        _ => false,
    }
}

/// Construct a provider client by name.
///
/// When `opts.default_model` is set, uses that model's catalog `api` + `base_url`.
pub fn create_provider(name: &str, opts: CreateProviderOpts) -> Result<Arc<dyn Provider>, String> {
    let p = canonicalize_provider(name);
    let timeout = if opts.timeout_secs == 0 {
        300
    } else {
        opts.timeout_secs
    };

    let sample = catalog_entry_for(&p, opts.default_model.as_deref());
    // Unknown provider without an explicit base URL would silently hit the
    // OpenAI endpoint with the wrong key — fail loudly instead. A custom
    // OpenAI-compatible endpoint stays available via `--base-url`.
    if sample.is_none() && opts.base_url.is_none() && p != "openai" {
        return Err(format!(
            "Unknown provider `{p}` — use a catalog provider \
             (anthropic / openai / openrouter) or pass --base-url \
             for a custom OpenAI-compatible endpoint."
        ));
    }
    let api = sample
        .map(|m| m.api.as_str())
        .unwrap_or("openai-completions");
    let catalog_url = opts
        .base_url
        .clone()
        .or_else(|| sample.map(|m| m.base_url.clone()).filter(|u| !u.is_empty()));

    match api {
        "openai-completions" | "openai-responses" | "mistral-conversations" => {
            let base = catalog_url
                .map(|u| normalize_openai_base(&u))
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            // For mistral catalog base is without /v1 path sometimes
            let base = if p == "mistral" && !base.contains("/v1") {
                format!("{}/v1", base.trim_end_matches('/'))
            } else {
                base
            };
            Ok(Arc::new(
                OpenAIProvider::new(
                    Some(base),
                    Some(p.clone()),
                    Some(api_key_env_for(&p).to_string()),
                )
                .with_timeout(timeout),
            ))
        }
        "anthropic-messages" => {
            let base = catalog_url
                .map(|u| normalize_anthropic_base(&u))
                .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
            Ok(Arc::new(
                AnthropicProvider::with_key_env(
                    Some(base),
                    Some(p.clone()),
                    Some(api_key_env_for(&p).to_string()),
                )
                .with_timeout(timeout),
            ))
        }
        other => Err(format!(
            "Provider `{}` uses API `{}` which rs-agent cannot drive yet. \
             Use anthropic / openai / openrouter or set a compatible base URL.",
            p, other
        )),
    }
}

/// Catalog models for providers that are both auth'd and runnable.
pub fn available_catalog_models() -> Vec<&'static CatalogModel> {
    let mut out = Vec::new();
    for p in catalog::catalog_providers() {
        if !supports_runtime(p) || !has_configured_auth(p) {
            continue;
        }
        out.extend(catalog::models_for_provider(p));
    }
    out
}

/// `provider/id` strings from the static catalog (ready providers only).
pub fn available_model_displays() -> Vec<String> {
    let mut out: Vec<String> = available_catalog_models()
        .into_iter()
        .map(|m| format!("{}/{}", m.provider, m.id))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Providers that currently have usable auth, each with a default model entry.
pub fn available_model_refs() -> Vec<ModelRef> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for m in available_catalog_models() {
        if seen.insert(m.provider.clone()) {
            out.push(ModelRef {
                provider: m.provider.clone(),
                model: m.id.clone(),
            });
        }
    }
    out
}

/// Human-readable auth status lines for `/provider`.
pub fn provider_status_lines() -> Vec<String> {
    let mut providers = known_providers();
    providers.retain(|p| supports_runtime(p));
    // Prefer showing runnable ones first
    providers.sort_by_key(|p| (!supports_runtime(p), p.clone()));
    providers
        .into_iter()
        .map(|p| {
            let runtime = if supports_runtime(&p) {
                "runnable"
            } else {
                "catalog-only"
            };
            let status = if has_configured_auth(&p) {
                "ready".to_string()
            } else {
                format!("missing {}", api_key_env_for(&p))
            };
            format!(
                "  {}  [{}] [{}]  default={}  ({} models)",
                p,
                status,
                runtime,
                default_model_for(&p),
                catalog::models_for_provider(&canonicalize_provider(&p)).len()
            )
        })
        .collect()
}

/// Fetch live model ids from every ready runnable provider; return as `provider/model`.
/// Merges on top of the static catalog in the TUI — live IDs that aren't catalogued still appear.
pub async fn fetch_all_model_displays(timeout_secs: u64) -> Vec<String> {
    let mut out = available_model_displays();
    for p in catalog::catalog_providers() {
        if !supports_runtime(p) || !has_configured_auth(p) {
            continue;
        }
        let Ok(provider) = create_provider(
            p,
            CreateProviderOpts {
                default_model: Some(default_model_for(p)),
                timeout_secs,
                ..Default::default()
            },
        ) else {
            continue;
        };
        let key = std::env::var(provider.api_key_env_var()).unwrap_or_default();
        if key.is_empty() {
            continue;
        }
        if let Ok(list) = provider.fetch_models(&key).await {
            for m in list {
                out.push(format!("{}/{}", p, m));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Env-mutating registry tests must not run in parallel.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn parse_bare_model_keeps_default_provider() {
        let r = ModelRef::parse("claude-sonnet-4-20250514", "anthropic");
        assert_eq!(r.provider, "anthropic");
        assert_eq!(r.model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn parse_colon_form() {
        let r = ModelRef::parse("openai:gpt-4o", "anthropic");
        assert_eq!(r.provider, "openai");
        assert_eq!(r.model, "gpt-4o");
    }

    #[test]
    fn unknown_slash_treated_as_model_id() {
        let r = ModelRef::parse("my/custom", "anthropic");
        assert_eq!(r.provider, "anthropic");
        assert_eq!(r.model, "my/custom");
    }

    #[test]
    fn provider_from_picker_row_extracts_id() {
        assert_eq!(
            provider_from_picker_row("openrouter  [needs key]  https://openrouter.ai/keys"),
            "openrouter"
        );
        assert_eq!(
            provider_from_picker_row("anthropic  [ready]  42 models"),
            "anthropic"
        );
    }

    #[test]
    fn connect_urls_for_core_providers() {
        assert!(provider_connect_url("anthropic").is_some());
        assert!(provider_connect_url("openrouter").is_some());
        assert!(provider_connect_url("openai").is_some());
    }

    #[test]
    fn create_core_providers() {
        for p in ["anthropic", "openai", "openrouter"] {
            assert!(
                create_provider(p, CreateProviderOpts::new()).is_ok(),
                "failed for {p}"
            );
        }
    }

    #[test]
    fn unknown_provider_without_base_url_errors() {
        // Removed provider ids must fail loudly, not silently hit OpenAI.
        assert!(create_provider("opencode", CreateProviderOpts::new()).is_err());
        assert!(create_provider("opencode-go", CreateProviderOpts::new()).is_err());
        assert!(create_provider("groq", CreateProviderOpts::new()).is_err());
        // A custom endpoint via --base-url still works for any name.
        let custom = CreateProviderOpts {
            base_url: Some("http://localhost:8080/v1".into()),
            ..Default::default()
        };
        assert!(create_provider("my-proxy", custom).is_ok());
    }

    #[test]
    fn core_providers_are_known_and_runnable() {
        for p in ["anthropic", "openai", "openrouter"] {
            assert!(is_known_provider(p), "{p} should be known");
            assert!(supports_runtime(p), "{p} should be runnable");
        }
    }

    #[test]
    fn catalog_hides_unkeyed_providers() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before_env = std::env::var_os("OPENAI_API_KEY");
        let before_home = std::env::var_os("HOME");
        std::env::remove_var("OPENAI_API_KEY");
        // Isolate from ~/.rs-agent/secrets.toml on the developer machine.
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", tmp.path());

        let d = available_model_displays();
        assert!(
            !d.iter().any(|x| x.starts_with("openai/")),
            "openai should be hidden without OPENAI_API_KEY or secrets"
        );

        if let Some(v) = before_env {
            std::env::set_var("OPENAI_API_KEY", v);
        }
        match before_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
