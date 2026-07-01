use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "rs-agent", version, about = "Minimalist AI agent toolkit")]
pub struct Cli {
    #[arg(long, default_value = "openai")]
    pub provider: String,

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

    #[arg(long, default_value = "false")]
    pub auto_mode: bool,
}
