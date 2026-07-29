use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "rs-agent", version, about = "Everyday coding agent with Deep Context")]
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

    /// Auto-approve read-only tools and file edits (write/edit); still prompt for
    /// bash/repl and non-readonly MCP tools. Distinct from `-a`/`--approve` (full YOLO).
    #[arg(long, default_value = "false")]
    pub auto_mode: bool,

    /// Output mode for -p runs: text (default) or json
    #[arg(long, default_value = "text")]
    pub mode: String,

    /// Max Deep Context recursion depth (root → child → leaf). Default 2.
    #[arg(long, default_value_t = 2)]
    pub rlm_depth: u32,

    /// Char threshold for auto Deep Context escalate hints on huge reads (0 = default 10000).
    #[arg(long, default_value_t = 0)]
    pub rlm_escalate_chars: usize,

    /// Extended thinking budget in tokens (Anthropic). 0 disables. Default: 10000 for anthropic, off otherwise.
    #[arg(long)]
    pub thinking_budget: Option<u32>,
}
