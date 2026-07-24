//! Provider/model registry — pi-style mid-session switching + static catalog.
//!
//! Active choice is a `(provider, model)` pair. The built-in catalog (from
//! reference/pi) lists ~1000 models; the picker shows catalog entries for
//! every provider that has configured auth and a runnable API client.

use crate::ai::anthropic::AnthropicProvider;
use crate::ai::bedrock::BedrockProvider;
use crate::ai::catalog::{self, CatalogModel};
use crate::ai::opencode_cli::OpenCodeCliProvider;
use crate::ai::openai::OpenAIProvider;
use crate::ai::provider::Provider;
use std::sync::Arc;

/// Hand-maintained extras not (yet) in the extracted pi catalog.
const EXTRA_PROVIDERS: &[&str] = &["opencode-cli", "bedrock"];

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

/// All provider ids we know about (catalog ∪ extras), sorted.
pub fn known_providers() -> Vec<String> {
    let mut v: Vec<String> = catalog::catalog_providers()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    for p in EXTRA_PROVIDERS {
        if !v.iter().any(|x| x == p) {
            // bedrock alias for amazon-bedrock
            v.push((*p).to_string());
        }
    }
    // Normalize: prefer `bedrock` as user-facing alias of amazon-bedrock
    if !v.iter().any(|x| x == "bedrock") {
        v.push("bedrock".to_string());
    }
    v.sort();
    v.dedup();
    v
}

/// Back-compat export used by help text.
pub const KNOWN_PROVIDERS: &[&str] = &[
    "anthropic",
    "openai",
    "openrouter",
    "opencode",
    "opencode-cli",
    "bedrock",
    "amazon-bedrock",
    "groq",
    "deepseek",
    "together",
    "fireworks",
    "xai",
    "mistral",
    "cerebras",
    "huggingface",
    "nvidia",
    "moonshotai",
    "vercel-ai-gateway",
    "google",
];

pub fn is_known_provider(name: &str) -> bool {
    let n = name.to_lowercase();
    if EXTRA_PROVIDERS.contains(&n.as_str()) || n == "bedrock" {
        return true;
    }
    catalog::catalog_providers()
        .iter()
        .any(|p| p.eq_ignore_ascii_case(&n))
}

fn canonicalize_provider(name: &str) -> String {
    match name.to_lowercase().as_str() {
        "bedrock" => "amazon-bedrock".to_string(),
        other => other.to_string(),
    }
}

pub fn default_model_for(provider: &str) -> String {
    let p = canonicalize_provider(provider);
    if p == "amazon-bedrock" {
        let models = catalog::models_for_provider(&p);
        // Prefer a US inference-profile Claude id — bare foundation IDs fail on Bedrock.
        if let Some(m) = models
            .iter()
            .find(|m| m.id.starts_with("us.anthropic.claude-sonnet"))
        {
            return m.id.clone();
        }
        if let Some(m) = models
            .iter()
            .find(|m| m.id.starts_with("us.anthropic."))
        {
            return m.id.clone();
        }
        return "us.anthropic.claude-opus-4-8".to_string();
    }
    if let Some(m) = catalog::models_for_provider(&p).first() {
        return m.id.clone();
    }
    match p.as_str() {
        "opencode-cli" => "opencode/deepseek-v4-flash-free".to_string(),
        _ => "gpt-4o".to_string(),
    }
}

pub fn api_key_env_for(provider: &str) -> &'static str {
    match canonicalize_provider(provider).as_str() {
        "openai" | "openai-codex" | "azure-openai-responses" => "OPENAI_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "opencode" | "opencode-go" | "opencode-cli" => "OPENCODE_API_KEY",
        "amazon-bedrock" => "AWS_ACCESS_KEY_ID",
        "groq" => "GROQ_API_KEY",
        "deepseek" => "DEEPSEEK_API_KEY",
        "together" => "TOGETHER_API_KEY",
        "fireworks" => "FIREWORKS_API_KEY",
        "xai" => "XAI_API_KEY",
        "mistral" => "MISTRAL_API_KEY",
        "cerebras" => "CEREBRAS_API_KEY",
        "huggingface" => "HF_TOKEN",
        "nvidia" => "NVIDIA_API_KEY",
        "moonshotai" | "moonshotai-cn" => "MOONSHOT_API_KEY",
        "vercel-ai-gateway" | "cloudflare-ai-gateway" => "AI_GATEWAY_API_KEY",
        "google" | "google-vertex" => "GEMINI_API_KEY",
        "github-copilot" => "GH_TOKEN",
        "minimax" | "minimax-cn" => "MINIMAX_API_KEY",
        "kimi-coding" => "KIMI_API_KEY",
        "zai" | "zai-coding-cn" => "ZAI_API_KEY",
        "xiaomi" | "xiaomi-token-plan-ams" | "xiaomi-token-plan-cn" | "xiaomi-token-plan-sgp" => {
            "XIAOMI_API_KEY"
        }
        "ant-ling" => "ANT_LING_API_KEY",
        "cloudflare-workers-ai" => "CLOUDFLARE_API_TOKEN",
        _ => "API_KEY",
    }
}

/// True when we can construct a runnable client for this provider's catalog API.
pub fn supports_runtime(provider: &str) -> bool {
    let p = canonicalize_provider(provider);
    if p == "opencode-cli" || p == "amazon-bedrock" {
        return true;
    }
    let models = catalog::models_for_provider(&p);
    let Some(sample) = models.first() else {
        return false;
    };
    matches!(
        sample.api.as_str(),
        "openai-completions"
            | "openai-responses"
            | "anthropic-messages"
            | "bedrock-converse-stream"
            | "mistral-conversations"
    )
}

pub fn has_configured_auth(provider: &str) -> bool {
    let p = canonicalize_provider(provider);
    match p.as_str() {
        "opencode-cli" => true,
        "amazon-bedrock" => {
            std::env::var("AWS_ACCESS_KEY_ID").is_ok()
                || std::env::var("AWS_PROFILE").is_ok()
                || std::path::Path::new(
                    &std::env::var("HOME")
                        .map(|h| format!("{}/.aws/credentials", h))
                        .unwrap_or_default(),
                )
                .exists()
        }
        other => {
            let env = api_key_env_for(other);
            if std::env::var(env).is_ok() {
                return true;
            }
            crate::config::Secrets::load().get_key(other).is_some()
        }
    }
}

/// Console / signup URL where the user can create an API key for `provider`.
pub fn provider_connect_url(provider: &str) -> Option<&'static str> {
    match canonicalize_provider(provider).as_str() {
        "anthropic" => Some("https://console.anthropic.com/settings/keys"),
        "openai" | "openai-codex" => Some("https://platform.openai.com/api-keys"),
        "openrouter" => Some("https://openrouter.ai/keys"),
        "groq" => Some("https://console.groq.com/keys"),
        "deepseek" => Some("https://platform.deepseek.com/api_keys"),
        "together" => Some("https://api.together.xyz/settings/api-keys"),
        "fireworks" => Some("https://fireworks.ai/account/api-keys"),
        "xai" => Some("https://console.x.ai/"),
        "mistral" => Some("https://console.mistral.ai/api-keys/"),
        "cerebras" => Some("https://cloud.cerebras.ai/"),
        "huggingface" => Some("https://huggingface.co/settings/tokens"),
        "nvidia" => Some("https://build.nvidia.com/settings/api-keys"),
        "google" | "google-vertex" => Some("https://aistudio.google.com/apikey"),
        "opencode" | "opencode-go" => Some("https://opencode.ai"),
        "vercel-ai-gateway" => Some("https://vercel.com/docs/ai-gateway"),
        "amazon-bedrock" => Some("https://console.aws.amazon.com/bedrock/"),
        "moonshotai" | "moonshotai-cn" => Some("https://platform.moonshot.ai/"),
        "minimax" | "minimax-cn" => Some("https://platform.minimax.io/"),
        "github-copilot" => Some("https://github.com/settings/tokens"),
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
        if !supports_runtime(&p) && p != "opencode-cli" {
            continue;
        }
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
            rows.push(format!(
                "{p}  [{mark}]  set {}",
                api_key_env_for(&p)
            ));
        } else {
            rows.push(format!("{p}  [{mark}]  {url}"));
        }
    }
    // Ensure core ones always appear even if catalog empty
    for p in ["anthropic", "openai", "openrouter", "opencode-cli", "bedrock"] {
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
    row.split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase()
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

fn catalog_base_url(provider: &str) -> Option<String> {
    catalog::models_for_provider(provider)
        .first()
        .map(|m| m.base_url.clone())
        .filter(|u| !u.is_empty())
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

/// Construct a provider client by name.
pub fn create_provider(
    name: &str,
    opts: CreateProviderOpts,
) -> Result<Arc<dyn Provider>, String> {
    let p = canonicalize_provider(name);
    let timeout = if opts.timeout_secs == 0 {
        300
    } else {
        opts.timeout_secs
    };

    if p == "opencode-cli" {
        return Ok(Arc::new(
            OpenCodeCliProvider::new(None, opts.default_model).with_timeout(timeout),
        ));
    }
    if p == "amazon-bedrock" {
        return Ok(Arc::new(BedrockProvider::new(opts.base_url, None)));
    }

    let sample = catalog::models_for_provider(&p).first().copied();
    let api = sample.map(|m| m.api.as_str()).unwrap_or("openai-completions");
    let catalog_url = opts
        .base_url
        .clone()
        .or_else(|| catalog_base_url(&p));

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
            Ok(Arc::new(OpenAIProvider::new(
                Some(base),
                Some(p.clone()),
                Some(api_key_env_for(&p).to_string()),
            )))
        }
        "anthropic-messages" => {
            let base = catalog_url
                .map(|u| normalize_anthropic_base(&u))
                .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
            Ok(Arc::new(AnthropicProvider::with_key_env(
                Some(base),
                Some(p.clone()),
                Some(api_key_env_for(&p).to_string()),
            )))
        }
        "bedrock-converse-stream" => Ok(Arc::new(BedrockProvider::new(opts.base_url, None))),
        other => Err(format!(
            "Provider `{}` uses API `{}` which rs-agent cannot drive yet. \
             Use openrouter / openai / anthropic / groq / … or set a compatible base URL.",
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
    // opencode-cli / bedrock aliases
    if has_configured_auth("opencode-cli") {
        // no catalog entries — synthetic default only via available_model_refs
    }
    if has_configured_auth("amazon-bedrock") || has_configured_auth("bedrock") {
        out.extend(catalog::models_for_provider("amazon-bedrock"));
    }
    out
}

/// `provider/id` strings from the static catalog (ready providers only).
pub fn available_model_displays() -> Vec<String> {
    let mut out: Vec<String> = available_catalog_models()
        .into_iter()
        .map(|m| format!("{}/{}", m.provider, m.id))
        .collect();
    if has_configured_auth("opencode-cli") {
        // Prefer live `opencode models` list; fall back to catalog `opencode/*` ids.
        let live = crate::ai::opencode_cli::list_opencode_models_blocking("opencode");
        if !live.is_empty() {
            for m in live {
                out.push(format!("opencode-cli/{}", m));
            }
        } else {
            for m in catalog::models_for_provider("opencode") {
                out.push(format!("opencode-cli/opencode/{}", m.id));
            }
            out.push(format!(
                "opencode-cli/{}",
                default_model_for("opencode-cli")
            ));
        }
    }
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
    if has_configured_auth("opencode-cli") && seen.insert("opencode-cli".into()) {
        out.push(ModelRef {
            provider: "opencode-cli".into(),
            model: default_model_for("opencode-cli"),
        });
    }
    out
}

/// Human-readable auth status lines for `/provider`.
pub fn provider_status_lines() -> Vec<String> {
    let mut providers = known_providers();
    providers.retain(|p| supports_runtime(p) || p == "google" || p == "github-copilot");
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
        if p == "opencode" && std::env::var("OPENCODE_API_KEY").is_err() {
            continue;
        }
        let key = std::env::var(provider.api_key_env_var()).unwrap_or_default();
        if let Ok(list) = provider.fetch_models(&key).await {
            for m in list {
                out.push(format!("{}/{}", p, m));
            }
        }
    }
    // Always refresh opencode-cli from the local CLI when present.
    if has_configured_auth("opencode-cli") {
        if let Ok(provider) = create_provider(
            "opencode-cli",
            CreateProviderOpts {
                default_model: Some(default_model_for("opencode-cli")),
                timeout_secs,
                ..Default::default()
            },
        ) {
            if let Ok(list) = provider.fetch_models("").await {
                for m in list {
                    out.push(format!("opencode-cli/{}", m));
                }
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

    #[test]
    fn parse_opencode_cli_nested_model_id() {
        let r = ModelRef::parse("opencode-cli/opencode/deepseek-v4-flash-free", "anthropic");
        assert_eq!(r.provider, "opencode-cli");
        assert_eq!(r.model, "opencode/deepseek-v4-flash-free");
    }

    #[test]
    fn parse_bare_model_keeps_default_provider() {
        let r = ModelRef::parse("claude-sonnet-4-20250514", "anthropic");
        assert_eq!(r.provider, "anthropic");
        assert_eq!(r.model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn parse_colon_form() {
        let r = ModelRef::parse("bedrock:us.anthropic.claude-opus-4-8", "openai");
        assert_eq!(r.provider, "bedrock");
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
        for p in ["anthropic", "openai", "openrouter", "groq", "opencode"] {
            assert!(
                create_provider(p, CreateProviderOpts::new()).is_ok(),
                "failed for {p}"
            );
        }
    }

    #[test]
    fn openrouter_is_known_and_runnable() {
        assert!(is_known_provider("openrouter"));
        assert!(supports_runtime("openrouter"));
    }

    #[test]
    fn catalog_hides_openrouter_without_its_key() {
        let before_env = std::env::var_os("OPENROUTER_API_KEY");
        let before_home = std::env::var_os("HOME");
        std::env::remove_var("OPENROUTER_API_KEY");
        // Isolate from ~/.rs-agent/secrets.toml on the developer machine.
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", tmp.path());

        let d = available_model_displays();
        assert!(
            !d.iter().any(|x| x.starts_with("openrouter/")),
            "openrouter should be hidden without OPENROUTER_API_KEY or secrets"
        );

        if let Some(v) = before_env {
            std::env::set_var("OPENROUTER_API_KEY", v);
        }
        match before_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
