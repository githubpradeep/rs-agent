use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "rs-agent", version, about = "RLM-style recursive coding agent")]
pub struct Cli {
    /// Provider: anthropic (recommended), openai, opencode, opencode-cli (experimental), bedrock.
    /// Omit to use the last selection from `~/.rs-agent/config.toml` (default: anthropic).
    #[arg(long)]
    pub provider: Option<String>,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub api_key: Option<String>,

    #[arg(long)]
    pub api_key_env: Option<String>,

    #[arg(long, default_value = "false")]
    pub stream: bool,

    #[arg(short = 'p', long)]
    pub prompt: Option<String>,

    #[arg(long)]
    pub base_url: Option<String>,

    #[arg(long, default_value_t = 300)]
    pub timeout: u64,

    #[arg(long, default_value = "false")]
    pub list_models: bool,

    /// YOLO mode: skip permission prompts entirely, auto-approving every tool call.
    #[arg(short = 'a', long, default_value = "false")]
    pub approve: bool,

    #[arg(short = 'r', long)]
    pub resume: Option<String>,

    #[arg(long, default_value = "false")]
    pub list_sessions: bool,

    #[arg(long, default_value = "false")]
    pub no_context_files: bool,

    #[arg(long)]
    pub system_prompt: Option<String>,

    #[arg(long)]
    pub append_system_prompt: Vec<String>,

    #[arg(long, default_value_t = 100)]
    pub max_iterations: usize,

    /// Lighter YOLO mode: auto-approve read-only tools (read/grep/ls/find/webfetch/websearch)
    /// without prompting; other tools still require permission.
    #[arg(long, default_value = "false")]
    pub auto_mode: bool,

    /// Output mode for -p runs: text (default) or json
    #[arg(long, default_value = "text")]
    pub mode: String,

    /// Max RLM recursion depth (root → child → leaf). Default 2.
    #[arg(long, default_value_t = 2)]
    pub rlm_depth: u32,

    /// Extended thinking budget in tokens (Anthropic). 0 disables. Default: 10000 for anthropic, off otherwise.
    #[arg(long)]
    pub thinking_budget: Option<u32>,
}
