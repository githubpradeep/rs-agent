use clap::Parser;
use rs_agent::ai::provider::Provider;
use rs_agent::ai::registry::{self, CreateProviderOpts};
use rs_agent::cli::Cli;
use rs_agent::config::Config;
use rs_agent::context;
use rs_agent::session::SessionStore;
use rs_agent::tui::App;
use std::io::Write;
use std::sync::Arc;

fn get_provider(name: &str, base_url: Option<&str>, default_model: Option<&str>, timeout_secs: u64) -> Result<Arc<dyn Provider>, String> {
    registry::create_provider(
        name,
        CreateProviderOpts {
            base_url: base_url.map(|s| s.to_string()),
            default_model: default_model.map(|s| s.to_string()),
            timeout_secs,
        },
    )
}

fn get_default_model(provider: &str) -> String {
    registry::default_model_for(provider)
}

/// Fill in CLI fields that are still at their clap defaults / `None` from
/// the loaded config. Any flag the user actually passed on the command
/// line is left untouched.
fn apply_config_defaults(cli: &mut Cli, cfg: &Config) {
    // Only fill when the user did not pass `--provider` / `--model`.
    if cli.provider.is_none() {
        cli.provider = cfg.provider.clone();
    }
    if cli.model.is_none() {
        cli.model = cfg.model.clone();
    }
    if cli.base_url.is_none() {
        cli.base_url = cfg.base_url.clone();
    }
    if cli.timeout == 300 {
        if let Some(t) = cfg.timeout {
            cli.timeout = t;
        }
    }
    if cli.max_iterations == 100 {
        if let Some(m) = cfg.max_iterations {
            cli.max_iterations = m;
        }
    }
    if cli.rlm_depth == 2 {
        if let Some(d) = cfg.rlm_depth {
            cli.rlm_depth = d;
        }
    }
    if cli.rlm_escalate_chars == 0 {
        if let Some(n) = cfg.rlm_escalate_chars {
            cli.rlm_escalate_chars = n;
        }
    }
    if cli.thinking_budget.is_none() {
        cli.thinking_budget = cfg.thinking_budget;
    }
    cli.approve = cli.approve || cfg.approve.unwrap_or(false);
    cli.auto_mode = cli.auto_mode || cfg.auto_mode.unwrap_or(false);
}

fn resolve_thinking_budget(cli: &Cli, provider: &dyn Provider) -> Option<u32> {
    match cli.thinking_budget {
        Some(0) => None,
        Some(n) => Some(n),
        None if provider.supports_thinking() => Some(10_000),
        None => None,
    }
}

fn read_line_prompt(prompt: &str) -> std::io::Result<String> {
    print!("{}", prompt);
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Interactive first-run setup when config has no provider and stdin is a TTY.
/// Skipped for `-p`, list modes, and non-interactive environments.
fn maybe_run_first_launch_wizard(cli: &mut Cli, cfg: &mut Config) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Ok(());
    }
    if cli.prompt.is_some() || cli.list_models || cli.list_sessions {
        return Ok(());
    }
    if !Config::user_config_needs_wizard() {
        return Ok(());
    }

    eprintln!("Welcome to rs-agent — first-launch setup");
    eprintln!("(Press Enter to accept defaults; Ctrl-C to cancel)\n");

    let provider = {
        let raw = read_line_prompt("Provider [anthropic/openai/opencode/opencode-cli/bedrock] (anthropic): ")?;
        if raw.is_empty() {
            "anthropic".to_string()
        } else {
            raw
        }
    };
    let default_model = get_default_model(&provider).to_string();
    let model = {
        let raw = read_line_prompt(&format!("Model ({}): ", default_model))?;
        if raw.is_empty() {
            default_model
        } else {
            raw
        }
    };
    let theme = {
        let raw = read_line_prompt("Theme [dark/light/forest] (dark): ")?;
        if raw.is_empty() {
            "dark".to_string()
        } else {
            raw
        }
    };

    cfg.provider = Some(provider.clone());
    cfg.model = Some(model.clone());
    cfg.theme = Some(theme);
    cfg.save_user_config()?;
    eprintln!("Wrote {}", Config::user_config_path().display());

    cli.provider = Some(provider.clone());
    if cli.model.is_none() {
        cli.model = Some(model);
    }

    let env_name = rs_agent::ai::registry::api_key_env_for(&provider).to_string();
    if !matches!(
        provider.to_lowercase().as_str(),
        "opencode-cli" | "bedrock" | "amazon-bedrock"
    ) && std::env::var(&env_name).is_err()
    {
        // OpenCode Zen: reuse local `auth.json` when present.
        if matches!(
            provider.to_lowercase().as_str(),
            "opencode" | "opencode-go"
        ) {
            rs_agent::ai::registry::export_opencode_auth_from_file();
        }
    }
    if !matches!(
        provider.to_lowercase().as_str(),
        "opencode-cli" | "bedrock" | "amazon-bedrock"
    ) && std::env::var(&env_name).is_err()
        && !rs_agent::ai::registry::has_configured_auth(&provider)
    {
        eprintln!("\nNo {} in the environment yet.", env_name);
        eprintln!("  export {}=...", env_name);
        if matches!(
            provider.to_lowercase().as_str(),
            "opencode" | "opencode-go"
        ) {
            eprintln!("  (or sign in with OpenCode — keys are read from ~/.local/share/opencode/auth.json)");
        }
        let smoke = read_line_prompt("Run a tiny smoke prompt after you set the key? [y/N]: ")?;
        if smoke.eq_ignore_ascii_case("y") || smoke.eq_ignore_ascii_case("yes") {
            eprintln!("Re-run rs-agent after exporting the key; smoke prompt: -p \"reply with pong\"");
        }
    } else if matches!(
        provider.to_lowercase().as_str(),
        "bedrock" | "amazon-bedrock"
    ) && std::env::var("AWS_ACCESS_KEY_ID").is_err()
        && !rs_agent::ai::registry::has_configured_auth("amazon-bedrock")
    {
        eprintln!("\nNo AWS credentials found yet.");
        eprintln!("  export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=...");
        eprintln!("  or configure ~/.aws/credentials ([default] or $AWS_PROFILE)");
    }

    eprintln!();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mut cli = Cli::parse();

    // Best-effort: create ~/.rs-agent (and subdirs) and seed a commented
    // example config on first run. Never fatal.
    let _ = Config::ensure_user_dir();
    let _ = Config::write_default_user_config_if_missing();
    rs_agent::config::export_secrets_to_env();
    rs_agent::ai::registry::export_opencode_auth_from_file();

    // Merge config file values into any CLI fields left at their clap
    // defaults / `None`. Explicit CLI flags always win.
    let mut cfg = Config::load();
    maybe_run_first_launch_wizard(&mut cli, &mut cfg)?;
    // Reload in case the wizard wrote new values (and re-apply).
    cfg = Config::load();
    apply_config_defaults(&mut cli, &cfg);
    let escalate = if cli.rlm_escalate_chars > 0 {
        cli.rlm_escalate_chars
    } else {
        rs_agent::agent::rlm_escalate::DEFAULT_ESCALATE_CHARS
    };
    rs_agent::agent::set_escalate_chars(escalate);

    let provider_name = cli
        .provider
        .clone()
        .unwrap_or_else(|| "anthropic".to_string());
    let model = {
        let m = cli
            .model
            .clone()
            .unwrap_or_else(|| get_default_model(&provider_name).to_string());
        cfg.resolve_model_alias(&m)
    };

    let provider = get_provider(
        &provider_name,
        cli.base_url.as_deref(),
        Some(model.as_str()),
        cli.timeout,
    )?;

    if provider_name.eq_ignore_ascii_case("opencode-cli") {
        std::env::set_var("OPENCODE_API_KEY", "cli-mode-no-key-needed");
    }

    if provider_name.eq_ignore_ascii_case("bedrock")
        || provider_name.eq_ignore_ascii_case("amazon-bedrock")
    {
        if std::env::var("AWS_ACCESS_KEY_ID").is_err() {
            rs_agent::ai::bedrock::export_credentials_from_file();
        }
    }

    // Re-export after provider construction path may have run (auth.json).
    if matches!(
        provider_name.to_lowercase().as_str(),
        "opencode" | "opencode-go"
    ) {
        rs_agent::ai::registry::export_opencode_auth_from_file();
    }

    let env_name = provider.api_key_env_var().to_string();
    if let Some(ref key) = cli.api_key {
        std::env::set_var(&env_name, key);
    }
    if let Some(ref alt_env) = cli.api_key_env {
        if let Ok(val) = std::env::var(alt_env) {
            std::env::set_var(&env_name, &val);
        } else {
            eprintln!("Warning: env var {} is not set", alt_env);
        }
    }

    let provider_lower = provider_name.to_lowercase();
    let skips_api_key_env = matches!(
        provider_lower.as_str(),
        "opencode-cli" | "bedrock" | "amazon-bedrock"
    );
    if !skips_api_key_env && std::env::var(&env_name).is_err() {
        eprintln!("Missing API key for {}.", provider_lower);
        eprintln!("  export {}=sk-...", env_name);
        if matches!(provider_lower.as_str(), "opencode" | "opencode-go") {
            eprintln!(
                "  Or sign in with OpenCode (reads ~/.local/share/opencode/auth.json)."
            );
        }
        eprintln!("Or paste a key via /provider|/login (saved to ~/.rs-agent/secrets.toml).");
        std::process::exit(1);
    }

    // Bedrock: env optional if ~/.aws/credentials (or AWS_PROFILE) is present.
    if matches!(provider_lower.as_str(), "bedrock" | "amazon-bedrock")
        && std::env::var("AWS_ACCESS_KEY_ID").is_err()
        && !rs_agent::ai::registry::has_configured_auth("amazon-bedrock")
    {
        eprintln!("Missing AWS credentials for Bedrock.");
        eprintln!("  export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=...");
        eprintln!("  or put keys in ~/.aws/credentials under [default] / $AWS_PROFILE");
        std::process::exit(1);
    }

    if cli.list_models {
        let api_key = std::env::var(provider.api_key_env_var()).unwrap_or_default();
        match provider.fetch_models(&api_key).await {
            Ok(list) => {
                println!("Available models for {}:", provider_name);
                for m in list {
                    println!("  {}", m);
                }
            }
            Err(e) => {
                eprintln!("Failed to fetch models: {:?}", e);
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    let base_prompt = cli.system_prompt.clone().unwrap_or_else(|| {
        rs_agent::agent::default_system_prompt()
    });

    let mut system_prompt = base_prompt;

    for arg in &cli.append_system_prompt {
        let resolved = context::resolve_append_arg(arg)
            .unwrap_or_else(|e| { eprintln!("Warning: {}", e); String::new() });
        if !resolved.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&resolved);
        }
    }

    if !cli.no_context_files {
        let context_files = context::discover_context_files();
        let context_section = context::build_context_section(&context_files);
        if !context_section.is_empty() {
            system_prompt.push_str(&context_section);
        }
        if !context_files.is_empty() {
            eprintln!("Loaded {} project context file(s)", context_files.len());
            for cf in &context_files {
                eprintln!("  {}", cf.path.display());
            }
        }

        let agent_commands = context::discover_agent_commands();
        let commands_section = context::build_commands_section(&agent_commands);
        if !commands_section.is_empty() {
            system_prompt.push_str(&commands_section);
        }
        if !agent_commands.is_empty() {
            eprintln!("Loaded {} agent command(s)", agent_commands.len());
            for cmd in &agent_commands {
                eprintln!("  {}", cmd.path.display());
            }
        }
    }

    if let Some(prompt) = &cli.prompt {
        let thinking_budget = resolve_thinking_budget(&cli, provider.as_ref());
        let mut agent = rs_agent::agent::AgentLoop::new(
            provider.clone(),
            rs_agent::agent::state::AgentState::new(model, provider_name.to_string())
                .with_system_prompt(system_prompt)
                .with_thinking_budget(thinking_budget),
        )
        .with_max_iterations(cli.max_iterations)
        .with_rlm_depth(0, cli.rlm_depth);
        rs_agent::tools::register_default_tools_with_rlm(&mut agent, cli.rlm_depth);
        {
            let mcp_cfg = rs_agent::config::Config::load().mcp;
            if !mcp_cfg.servers.is_empty() {
                let lines = rs_agent::mcp::attach_mcp_from_config(&mut agent, &mcp_cfg).await;
                for line in &lines {
                    eprintln!("{line}");
                }
            }
        }

        let json_mode = cli.mode.eq_ignore_ascii_case("json");
        let mut has_error = false;
        agent
            .run(prompt, &mut |event| {
                if json_mode {
                    let v = match &event {
                        rs_agent::agent::AgentEvent::TextDelta { text } => {
                            serde_json::json!({"type":"text","text": text})
                        }
                        rs_agent::agent::AgentEvent::ThinkingDelta { thinking } => {
                            serde_json::json!({"type":"thinking","text": thinking})
                        }
                        rs_agent::agent::AgentEvent::ToolUseStart { id, name } => {
                            serde_json::json!({"type":"tool_start","id": id, "name": name})
                        }
                        rs_agent::agent::AgentEvent::ToolResult { id, name, result } => {
                            serde_json::json!({"type":"tool_result","id": id, "name": name, "content": result.content, "is_error": result.is_error})
                        }
                        rs_agent::agent::AgentEvent::Error { message } => {
                            has_error = true;
                            serde_json::json!({"type":"error","message": message})
                        }
                        rs_agent::agent::AgentEvent::Status { message } => {
                            serde_json::json!({"type":"status","message": message})
                        }
                        rs_agent::agent::AgentEvent::Done => serde_json::json!({"type":"done"}),
                        rs_agent::agent::AgentEvent::Aborted => serde_json::json!({"type":"aborted"}),
                        rs_agent::agent::AgentEvent::TreeUpdate { tree } => {
                            serde_json::json!({"type":"tree","tree": tree.snapshot()})
                        }
                        _ => return,
                    };
                    println!("{}", v);
                } else {
                    match event {
                        rs_agent::agent::AgentEvent::TextDelta { text } => {
                            print!("{}", text);
                            std::io::stdout().flush().ok();
                        }
                        rs_agent::agent::AgentEvent::ThinkingDelta { thinking } => {
                            eprint!("{}", thinking);
                            std::io::stderr().flush().ok();
                        }
                        rs_agent::agent::AgentEvent::Error { message } => {
                            eprintln!("\n[error] {}", message);
                            has_error = true;
                        }
                        rs_agent::agent::AgentEvent::Status { message } => {
                            eprintln!("[{}]", message);
                        }
                        _ => {}
                    }
                }
            })
            .await
            .map_err(|e| e.to_string())?;

        if !json_mode {
            println!();
            if !has_error {
                println!("--- Final messages ---");
                for msg in &agent.state().messages {
                    let role = match msg.role {
                        rs_agent::ai::types::Role::User => "user",
                        rs_agent::ai::types::Role::Assistant => "assistant",
                        rs_agent::ai::types::Role::Tool => "tool",
                        rs_agent::ai::types::Role::System => "system",
                    };
                    for content in &msg.content {
                        match &content.text {
                            Some(text) if !text.is_empty() => println!("[{}] {}", role, text),
                            _ => {}
                        }
                    }
                }
            }
        } else {
            println!(
                "{}",
                serde_json::json!({
                    "type": "tree_final",
                    "tree": agent.call_tree().snapshot(),
                })
            );
        }

        return Ok(());
    }

    let store = SessionStore::new();

    if cli.list_sessions {
        match store.list() {
            Ok(sessions) => {
                if sessions.is_empty() {
                    println!("No saved sessions.");
                } else {
                    println!("Saved sessions:");
                    for id in &sessions {
                        if let Ok(data) = store.load(id) {
                            let msgs = data.messages.len();
                            println!("  {}  ({} messages, {})", id, msgs, data.model);
                        }
                    }
                }
            }
            Err(e) => eprintln!("Error listing sessions: {}", e),
        }
        return Ok(());
    }

    let resume_session = cli.resume.as_ref().and_then(|id| {
        store.load(id).map_err(|e| eprintln!("{}", e)).ok()
    });

    let thinking_budget = resolve_thinking_budget(&cli, provider.as_ref());
    let mut app = App::new(
        provider,
        model,
        cli.timeout,
        cli.approve,
        resume_session,
        Some(system_prompt),
        cli.max_iterations,
        cli.auto_mode,
        cli.rlm_depth,
        thinking_budget,
    );
    app.run()?;

    Ok(())
}
