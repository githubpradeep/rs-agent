use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "rs-agent",
    version,
    about = "Overnight coding factory with Deep Context"
)]
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

    #[arg(
        short = 'r',
        long,
        value_name = "ID",
        help = "Resume a session: full id, date suffix (20260808_113045), prefix, or latest/last/-"
    )]
    pub resume: Option<String>,

    /// List saved sessions (ids + titles) for `-r`.
    #[arg(long, default_value = "false")]
    pub list_sessions: bool,

    #[arg(long, default_value = "false")]
    pub no_context_files: bool,

    #[arg(long)]
    pub system_prompt: Option<String>,

    #[arg(long)]
    pub append_system_prompt: Vec<String>,

    #[arg(long, default_value_t = 99999)]
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

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Overnight factory: claim ready beads and implement until empty or budget.
    Worker(WorkerArgs),
    /// Reclaim stale leases, dead pids, auto-assign, stuck mail.
    Marshal(MarshalArgs),
    /// Start/stop/inspect multiple workers (Phase B).
    Fleet(FleetArgs),
    /// Wish Factory — intake a wish as a design/task bead.
    Wish(WishArgs),
    /// Standing role runner (Beadle, Gargoyle, …).
    Role(RoleArgs),
    /// Lifecycle status / wait (herdr agent.wait).
    Status(StatusArgs),
    /// JSON control-plane client / daemon (herdr socket API).
    Api(ApiArgs),
    /// Headless runtime daemon (reattachable control plane).
    Runtime(RuntimeArgs),
    /// Cron schedules for marshal/worker wake.
    Schedule(ScheduleArgs),
    /// Emit implement beads from a planner bullet list (PLAN_EXECUTE MVP).
    PlanExecute {
        /// Plan text (lines become beads).
        text: Vec<String>,
    },
}

#[derive(Parser, Debug)]
pub struct StatusArgs {
    #[command(subcommand)]
    pub command: Option<StatusCommand>,
}

#[derive(Subcommand, Debug)]
pub enum StatusCommand {
    /// Print current lifecycle snapshot.
    Show,
    /// Block until lifecycle matches.
    Wait {
        /// blocked | idle | done | working
        #[arg(long, default_value = "blocked")]
        until: String,
        #[arg(long, default_value_t = 30)]
        timeout_secs: u64,
    },
    /// Resume a seat blocked on human input (headless HUMAN wait).
    Resume {
        /// Fleet seat name.
        #[arg(long)]
        seat: String,
        /// Typed answer / approve note written into control.jsonl.
        #[arg(long, default_value = "approved")]
        answer: String,
    },
}

#[derive(Parser, Debug)]
pub struct ApiArgs {
    /// Method name (ping, agent.status, agent.wait, …).
    pub method: String,
    /// JSON params object.
    #[arg(long, default_value = "{}")]
    pub params: String,
    /// Socket path (default ~/.rs-agent/rs-agent.sock).
    #[arg(long)]
    pub socket: Option<String>,
}

#[derive(Parser, Debug)]
pub struct RuntimeArgs {
    #[command(subcommand)]
    pub command: RuntimeCommand,
}

#[derive(Subcommand, Debug)]
pub enum RuntimeCommand {
    /// Listen on the control socket until stop.
    Serve {
        #[arg(long)]
        socket: Option<String>,
    },
    /// Request daemon stop.
    Stop {
        #[arg(long)]
        socket: Option<String>,
    },
}

#[derive(Parser, Debug)]
pub struct ScheduleArgs {
    #[command(subcommand)]
    pub command: ScheduleCommand,
}

#[derive(Subcommand, Debug)]
pub enum ScheduleCommand {
    /// List schedules.
    List,
    /// Add a cron schedule.
    Add {
        name: String,
        /// Five-field cron: min hour dom month dow
        cron: String,
        command: String,
    },
    /// Show schedules due this minute.
    Due,
}

#[derive(Parser, Debug)]
pub struct WorkerArgs {
    /// Claim at most one ready bead then exit (default when --loop is off).
    #[arg(long, default_value = "false")]
    pub once: bool,

    /// Keep polling for ready beads until budget expires.
    #[arg(long, default_value = "false")]
    pub r#loop: bool,

    /// Wall-clock budget in minutes (default 480).
    #[arg(long, default_value_t = 480)]
    pub budget_minutes: u64,

    /// Claimant / seat name (default worker-<pid>). Loads seat model/provider if set.
    #[arg(long)]
    pub seat: Option<String>,

    /// Exit on first non-transport failure.
    #[arg(long, default_value = "false")]
    pub fail_fast: bool,

    /// Seconds to sleep when idle / after recoverable errors.
    #[arg(long, default_value_t = 5)]
    pub sleep_secs: u64,

    /// Log tools/text to stderr and `.rs-agent/fleet/<seat>.log` (default on).
    #[arg(long, default_value = "true")]
    pub verbose: bool,

    /// Quiet mode — status lines only (disables --verbose).
    #[arg(long, default_value = "false")]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct MarshalArgs {
    /// Run once and exit (default).
    #[arg(long, default_value = "true")]
    pub once: bool,

    /// Keep reclaiming/assigning until budget.
    #[arg(long, default_value = "false")]
    pub r#loop: bool,

    /// Assign bead to seat: `--assign b12 --seat Fleet-1`
    #[arg(long)]
    pub assign: Option<String>,

    #[arg(long)]
    pub seat: Option<String>,

    /// Disable auto-assign of implement beads to idle fleet.
    #[arg(long, default_value = "false")]
    pub no_auto_assign: bool,

    #[arg(long, default_value_t = 60)]
    pub interval_secs: u64,

    #[arg(long, default_value_t = 480)]
    pub budget_minutes: u64,

    /// Minutes before blocked/stuck claims get mailed (0 = skip).
    #[arg(long, default_value_t = 45)]
    pub stuck_mins: u64,
}

#[derive(Parser, Debug)]
pub struct FleetArgs {
    #[command(subcommand)]
    pub command: FleetCommand,
}

#[derive(Subcommand, Debug)]
pub enum FleetCommand {
    /// Spawn workers for each seat (detached).
    Up {
        /// Comma/space separated seat names (default Fleet-1,Fleet-2).
        #[arg(long, default_value = "Fleet-1,Fleet-2")]
        seats: String,
        /// Wall-clock budget minutes per worker.
        #[arg(long, default_value_t = 480)]
        budget_minutes: u64,
        #[arg(long, default_value_t = 5)]
        sleep_secs: u64,
        /// Quiet worker logs.
        #[arg(long, default_value = "false")]
        quiet: bool,
        #[arg(long, default_value = "false")]
        fail_fast: bool,
        /// Run all seats in this checkout (they can overwrite each other). Default: one git worktree per seat.
        #[arg(long, default_value = "false")]
        shared_worktree: bool,
    },
    /// Stop fleet workers (all, or --seats …).
    Down {
        #[arg(long)]
        seats: Option<String>,
    },
    /// Print live fleet + backlog status.
    Status,
    /// Tail a seat's log.
    Logs {
        seat: String,
        #[arg(long, default_value_t = 60)]
        lines: usize,
    },
}

#[derive(Parser, Debug)]
pub struct WishArgs {
    /// Wish text (or pass as trailing args).
    pub text: Vec<String>,
    /// Create as task instead of design.
    #[arg(long, default_value = "false")]
    pub task: bool,
    /// Mark ready for overnight (lower priority number).
    #[arg(long, default_value = "false")]
    pub auto: bool,
}

#[derive(Parser, Debug)]
pub struct RoleArgs {
    /// Role name: Beadle | Gargoyle | Drawbridge | Scryer | Marshal
    #[arg(long)]
    pub seat: Option<String>,
    /// Alias for --seat when positional feels better.
    pub role: Option<String>,
    #[arg(long, default_value = "false")]
    pub once: bool,
    #[arg(long, default_value = "false")]
    pub r#loop: bool,
    #[arg(long, default_value_t = 300)]
    pub interval_secs: u64,
    #[arg(long, default_value_t = 480)]
    pub budget_minutes: u64,
    /// Optional source path/URL for Scryer.
    #[arg(long)]
    pub source: Option<String>,
}
