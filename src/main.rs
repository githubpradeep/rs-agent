use clap::Parser;
use rs_agent::ai::anthropic::AnthropicProvider;
use rs_agent::ai::bedrock::BedrockProvider;
use rs_agent::ai::opencode_cli::OpenCodeCliProvider;
use rs_agent::ai::openai::OpenAIProvider;
use rs_agent::ai::provider::Provider;
use rs_agent::cli::Cli;
use rs_agent::tui::App;
use std::io::Write;
use std::sync::Arc;

fn get_provider(name: &str, base_url: Option<&str>, default_model: Option<&str>) -> Result<Arc<dyn Provider>, String> {
    match name.to_lowercase().as_str() {
        "openai" => Ok(Arc::new(OpenAIProvider::new(
            base_url.map(|s| s.to_string()),
            None,
            None,
        ))),
        "anthropic" => Ok(Arc::new(AnthropicProvider::new(
            base_url.map(|s| s.to_string()),
            None,
        ))),
        "opencode" => Ok(Arc::new(OpenAIProvider::new(
            Some(
                base_url
                    .unwrap_or("https://opencode.ai/zen/v1")
                    .to_string(),
            ),
            Some("opencode".to_string()),
            Some("OPENCODE_API_KEY".to_string()),
        ))),
        "opencode-cli" => Ok(Arc::new(OpenCodeCliProvider::new(
            None,
            default_model.map(|s| s.to_string()),
        ))),
        "bedrock" => Ok(Arc::new(BedrockProvider::new(
            base_url.map(|s| s.to_string()),
            None,
        ))),
        _ => Err(format!(
            "Unknown provider: {}. Supported: openai, anthropic, opencode, opencode-cli, bedrock",
            name
        )),
    }
}

fn get_default_model(provider: &str) -> &str {
    match provider {
        "anthropic" => "claude-sonnet-4-20250514",
        "bedrock" => "us.anthropic.claude-opus-4-8",
        "opencode" | "opencode-cli" => "opencode/deepseek-v4-flash-free",
        _ => "gpt-4o",
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let provider_name = &cli.provider;
    let model = cli
        .model
        .clone()
        .unwrap_or_else(|| get_default_model(provider_name).to_string());

    let provider = get_provider(provider_name, cli.base_url.as_deref(), cli.model.as_deref())?;

    if cli.provider.to_lowercase() == "opencode-cli" {
        std::env::set_var("OPENCODE_API_KEY", "cli-mode-no-key-needed");
    }

    if cli.provider.to_lowercase() == "bedrock" {
        if std::env::var("AWS_ACCESS_KEY_ID").is_err() {
            rs_agent::ai::bedrock::export_credentials_from_file();
        }
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

    if let Some(prompt) = &cli.prompt {
        let mut agent = rs_agent::agent::AgentLoop::new(
            provider.clone(),
            rs_agent::agent::state::AgentState::new(model, provider_name.to_string())
                .with_system_prompt(
                    "You are an expert coding assistant operating inside rs-agent, a coding agent harness. \
                     You help users by reading files, executing commands, editing code, and writing new files.\n\n\
                     Guidelines:\n\
                     - Use `read` to examine files instead of cat or sed.\n\
                     - Use `bash` to execute commands. Prefer bash over read for file listing (ls, find).\n\
                     - Use `edit` for precise changes to existing files.\n\
                     - Use `write` to create new files or complete rewrites.\n\
                     - Use `grep` to search for patterns in the codebase.\n\
                     - When writing code, first understand the patterns, then implement, then test.\n\
                     - Always check if the code compiles/runs correctly after making changes."
                    .to_string(),
                ),
        );
        rs_agent::tools::register_default_tools(&mut agent);

        let mut has_error = false;
        agent
            .run(prompt, &mut |event| {
                match event {
                    rs_agent::agent::AgentEvent::TextDelta { text } => {
                        print!("{}", text);
                        std::io::stdout().flush().ok();
                    }
                    rs_agent::agent::AgentEvent::Error { message } => {
                        eprintln!("\n[error] {}", message);
                        has_error = true;
                    }
                    _ => {}
                }
            })
            .await
            .map_err(|e| e.to_string())?;

        println!();
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

        return Ok(());
    }

    let mut app = App::new(provider, model, cli.timeout, cli.approve);
    app.run()?;

    Ok(())
}
