//! Slash-command dispatch and user-text submission (dir-token
//! expansion, system-prompt rebuild).
//!
use super::helpers::*;
use super::*;

impl App {
    pub(super) fn handle_slash_command(&mut self, text: &str) -> bool {
        let trimmed = text.trim();
        if !trimmed.starts_with('/') {
            return false;
        }
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim();
        match cmd {
            "/help" => {
                self.push_system(format!(
                    "Commands:\n\
                     /help /keys /settings /clear /context [on|off] /commands /tree\n\
                     /skills  /skill <name>  /prompt|/p <name> [args]  /reload\n\
                     /mode plan|ask|agent  /model [provider/model]  /provider|/login [name]\n\
                     /goal [condition|clear|pause|resume]  /theme [dark|light|forest]\n\
                     /recall <id>  /compact  /new  /fork [@N] [label]  /timeline  /sessions\n\
                     /export [md|json|html]  /image [path]  /lsp [start|stop|status]\n\
                     /revert  /trust list|reset  /rename <title>  /history [query|n]\n\n\
                     Keys: {}\n\
                     Ctrl+P cycle model · Tab-complete /skill|/prompt|/model|/theme|/mode|/provider · @ file · # dir",
                    self.keys.hint_line(),
                ));
            }
            "/keys" => {
                self.push_system(format!(
                    "Keymap\n\
                     {insert}          insert mode\n\
                     Esc        normal / abort when waiting\n\
                     Enter      send (insert) / steer (waiting)\n\
                     Up/Down    history (insert) / scroll (normal)\n\
                     {thinking}          toggle last thinking block\n\
                     {expand}          toggle last tool result block\n\
                     {tree}          toggle the live call-tree side panel\n\
                     {bottom}          jump to bottom\n\
                     @          file picker\n\
                     #          directory picker (attach dir summary)\n\
                     Tab        complete /skill /prompt /model /theme /mode /provider\n\
                     ^P         cycle provider/model (ready providers)\n\
                     {once}/{deny}/{always}/{path}  once / deny / trust project / allow path\n\
                     ^C         quit\n\n\
                     Remap single-key actions in ~/.rs-agent/config.toml under [keybindings].\n\
                     /model = model picker · /provider|/login = connect provider (paste key).",
                    insert = self.keys.binding("insert"),
                    thinking = self.keys.binding("toggle_thinking"),
                    expand = self.keys.binding("expand_tool"),
                    tree = self.keys.binding("toggle_tree"),
                    bottom = self.keys.binding("jump_bottom"),
                    once = self.keys.binding("perm_once"),
                    deny = self.keys.binding("perm_deny"),
                    always = self.keys.binding("perm_always"),
                    path = self.keys.binding("perm_path"),
                ));
            }
            "/theme" => {
                if arg.is_empty() {
                    self.push_system(format!(
                        "Current theme: {}\nUsage: /theme dark|light|forest|auto",
                        self.theme_name.as_str()
                    ));
                } else {
                    self.theme_name = ThemeName::parse(arg);
                    self.palette = Palette::for_theme(self.theme_name);
                    self.push_system(format!("Theme set to {}", self.theme_name.as_str()));
                }
            }
            "/settings" => {
                self.settings = Some(SettingsState::from_app(
                    self.theme_name,
                    self.mouse_enabled,
                    self.toast_enabled,
                    self.toast_sound,
                ));
            }
            "/history" => {
                if arg.is_empty() {
                    if self.input_history.is_empty() {
                        self.push_system("No input history yet.");
                    } else {
                        let total = self.input_history.len();
                        let start = total.saturating_sub(20);
                        let mut out = String::from("Input history (most recent, newest last):\n");
                        for (i, entry) in self.input_history.iter().enumerate().skip(start) {
                            let preview: String = entry.chars().take(100).collect();
                            out.push_str(&format!(
                                "  {:>3}  {}\n",
                                i + 1,
                                preview.replace('\n', " ⏎ ")
                            ));
                        }
                        out.push_str(
                            "\n/history <query> to search · /history <n> to edit that entry",
                        );
                        self.push_system(out);
                    }
                } else if let Ok(n) = arg.parse::<usize>() {
                    if n >= 1 && n <= self.input_history.len() {
                        self.input = self.input_history[n - 1].clone();
                        self.history_index = None;
                        self.status = format!("history #{} loaded into input — edit & Enter", n);
                    } else {
                        self.push_system(format!(
                            "No history entry #{} (have {}).",
                            n,
                            self.input_history.len()
                        ));
                    }
                } else {
                    let query = arg.to_lowercase();
                    let matches: Vec<(usize, &String)> = self
                        .input_history
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| e.to_lowercase().contains(&query))
                        .collect();
                    if matches.is_empty() {
                        self.push_system(format!("No history matches for `{}`.", arg));
                    } else {
                        let mut out = format!("History matches for `{}`:\n", arg);
                        for (i, entry) in matches.iter().take(20) {
                            let preview: String = entry.chars().take(100).collect();
                            out.push_str(&format!(
                                "  {:>3}  {}\n",
                                i + 1,
                                preview.replace('\n', " ⏎ ")
                            ));
                        }
                        out.push_str("\n/history <n> to edit an entry");
                        self.push_system(out);
                    }
                }
            }
            "/rename" => {
                if arg.is_empty() {
                    self.push_system(format!(
                        "Current title: {}\nUsage: /rename <title>",
                        self.session_title
                            .clone()
                            .unwrap_or_else(|| "(untitled)".to_string())
                    ));
                } else {
                    self.session_title = Some(arg.to_string());
                    let _ = self.command_tx.send(AppCommand::SetTitle {
                        title: arg.to_string(),
                    });
                    self.push_system(format!("Session renamed to \"{}\"", arg));
                }
            }
            "/clear" => {
                let banner = self.messages.first().cloned();
                self.messages.clear();
                if let Some(b) = banner {
                    if b.role == "system" {
                        self.messages.push(b);
                    }
                }
                self.goal_indicator.clear();
                let _ = self.command_tx.send(AppCommand::GoalClear);
                self.push_system("Chat cleared (session kept). Use /new for a fresh session.");
            }
            "/goal" => {
                match crate::agent::parse_goal_arg(arg) {
                    Ok(crate::agent::GoalCommand::Status) => {
                        let _ = self.command_tx.send(AppCommand::GoalStatus);
                    }
                    Ok(crate::agent::GoalCommand::Clear) => {
                        self.goal_indicator.clear();
                        let _ = self.command_tx.send(AppCommand::GoalClear);
                    }
                    Ok(crate::agent::GoalCommand::Pause) => {
                        let _ = self.command_tx.send(AppCommand::GoalPause);
                    }
                    Ok(crate::agent::GoalCommand::Resume) => {
                        let _ = self.command_tx.send(AppCommand::GoalResume);
                    }
                    Ok(crate::agent::GoalCommand::Set(condition)) => {
                        self.goal_indicator = "◎ /goal active".into();
                        let _ = self.command_tx.send(AppCommand::GoalSet {
                            condition: condition.clone(),
                        });
                        self.push_system(format!("◎ /goal set — {condition}"));
                        // Claude Code: setting a goal starts a turn immediately.
                        self.submit_user_text(condition);
                    }
                    Err(e) => self.push_system(e),
                }
            }
            "/context" => match arg.to_lowercase().as_str() {
                "on" | "off" | "toggle" => {
                    self.context_enabled = match arg.to_lowercase().as_str() {
                        "on" => true,
                        "off" => false,
                        _ => !self.context_enabled,
                    };
                    let prompt = self.rebuild_system_prompt();
                    let _ = self.command_tx.send(AppCommand::SetSystemPrompt { prompt });
                    self.push_system(format!(
                        "Project context inclusion: {}. System prompt rebuilt.",
                        if self.context_enabled { "on" } else { "off" }
                    ));
                }
                _ => {
                    let files = discover_context_files();
                    let cmds = discover_agent_commands();
                    let mut out = format!(
                        "Context inclusion: {} (/context on|off to toggle)\n\nLoaded context:\n",
                        if self.context_enabled { "on" } else { "off" }
                    );
                    if files.is_empty() {
                        out.push_str("  (no AGENTS.md / CLAUDE.md)\n");
                    } else {
                        for f in &files {
                            out.push_str(&format!(
                                "  {} ({} chars)\n",
                                f.path.display(),
                                f.content.len()
                            ));
                        }
                    }
                    out.push_str("Commands (.rs-agent/commands):\n");
                    if cmds.is_empty() {
                        out.push_str("  (none)\n");
                    } else {
                        for c in &cmds {
                            out.push_str(&format!(
                                "  {} ({} chars)\n",
                                c.path.display(),
                                c.content.len()
                            ));
                        }
                    }
                    self.push_system(out);
                }
            },
            "/commands" => {
                let cmds = discover_agent_commands();
                if cmds.is_empty() {
                    self.push_system("No project commands in .rs-agent/commands/");
                } else {
                    let mut out = String::from("Project commands:\n");
                    for c in &cmds {
                        let name = c.path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                        out.push_str(&format!("  /{} — {}\n", name, c.path.display()));
                    }
                    self.push_system(out);
                }
            }
            "/skills" => {
                let skills = discover_skills();
                self.push_system(list_skills_summary(&skills));
            }
            "/skill" => {
                if arg.is_empty() {
                    self.push_system("Usage: /skill <name>");
                } else {
                    let skills = discover_skills();
                    match find_skill(&skills, arg) {
                        Some(skill) => {
                            let injection = format_skill_injection(skill);
                            let tools_note = if skill.tools.is_empty() {
                                String::new()
                            } else {
                                format!(" (tools: {})", skill.tools.join(", "))
                            };
                            let _ = self.command_tx.send(AppCommand::SetSkillTools {
                                tools: skill.tools.clone(),
                            });
                            self.push_system(format!(
                                "Loaded skill `{}`{}",
                                skill.name, tools_note
                            ));
                            self.submit_user_text(format!(
                                "{}\n\nApply this skill to the current task. If no task was given yet, briefly confirm you loaded it and wait for instructions.",
                                injection
                            ));
                        }
                        None => self.push_system(format!("Unknown skill `{}`. Try /skills.", arg)),
                    }
                }
            }
            "/prompt" | "/p" => {
                let mut rest = arg.splitn(2, char::is_whitespace);
                let name = rest.next().unwrap_or("").trim();
                let args = rest.next().unwrap_or("").trim();
                if name.is_empty() {
                    let templates = discover_templates();
                    if templates.is_empty() {
                        self.push_system(
                            "No templates. Add markdown under ~/.rs-agent/prompts/ or .rs-agent/prompts/",
                        );
                    } else {
                        let mut out = String::from("Templates:\n");
                        for t in &templates {
                            out.push_str(&format!("  {} — {}\n", t.name, t.description));
                        }
                        self.push_system(out);
                    }
                } else {
                    let templates = discover_templates();
                    match find_template(&templates, name) {
                        Some(t) => {
                            self.input = render_template(t, args);
                            self.input_mode = InputMode::Insert;
                            self.status = format!("template `{}` in input — edit & Enter", t.name);
                        }
                        None => self.push_system(format!("Unknown template `{}`", name)),
                    }
                }
            }
            "/reload" => {
                let n = discover_skills().len();
                let t = discover_templates().len();
                let prompt = self.rebuild_system_prompt();
                let _ = self.command_tx.send(AppCommand::SetSystemPrompt { prompt });
                self.push_system(format!(
                    "Reloaded {} skill(s), {} template(s); system prompt rebuilt.",
                    n, t
                ));
            }
            "/mode" => {
                if arg.is_empty() {
                    self.push_system(format!(
                        "Current mode: {}\nUsage: /mode plan|ask|agent",
                        self.agent_mode.as_str()
                    ));
                } else if let Some(mode) = AgentMode::parse(arg) {
                    self.agent_mode = mode;
                    let _ = self.command_tx.send(AppCommand::SetMode { mode });
                    self.push_system(format!("Mode set to {}", mode.as_str()));
                } else {
                    self.push_system("Usage: /mode plan|ask|agent");
                }
            }
            "/sessions" => {
                let raw = arg.trim().to_lowercase();
                if raw == "list" || raw == "text" {
                    match SessionStore::new().list_summaries() {
                        Ok(mut summaries) => {
                            if summaries.is_empty() {
                                self.push_system("No saved sessions.");
                            } else {
                                let project = SessionStore::current_project_root();
                                summaries.sort_by(|a, b| b.id.cmp(&a.id));
                                let mut out = String::from("Sessions (newest first):\n");
                                for s in summaries.iter().take(30) {
                                    let title =
                                        s.title.clone().unwrap_or_else(|| "(untitled)".to_string());
                                    let mine = project
                                        .as_ref()
                                        .map(|p| {
                                            SessionStore::same_project(s.project_root.as_deref(), p)
                                        })
                                        .unwrap_or(false);
                                    let mark = if s.id == self.session_id {
                                        "●"
                                    } else if mine {
                                        "·"
                                    } else {
                                        " "
                                    };
                                    let fork = match (&s.parent_id, &s.branch_label) {
                                        (Some(p), Some(l)) => {
                                            format!(" ↩{} [{}]", SessionStore::short_id(p), l)
                                        }
                                        (Some(p), None) => {
                                            format!(" ↩{}", SessionStore::short_id(p))
                                        }
                                        _ => String::new(),
                                    };
                                    out.push_str(&format!(
                                        "  {mark} {}  {} msgs  {}  — {}{}\n",
                                        SessionStore::short_id(&s.id),
                                        s.message_count,
                                        s.model,
                                        title,
                                        fork
                                    ));
                                }
                                out.push_str(
                                    "Panel: /sessions · Switch in-panel with Enter · New: n or /new",
                                );
                                self.push_system(out);
                            }
                        }
                        Err(e) => self.push_system(format!("Failed to list sessions: {}", e)),
                    }
                } else {
                    self.toggle_sessions_panel();
                }
            }
            "/export" => {
                let fmt = if arg.is_empty() {
                    "md"
                } else {
                    match arg.trim().to_lowercase().as_str() {
                        "md" | "markdown" => "md",
                        "json" => "json",
                        "html" | "htm" => "html",
                        other => {
                            self.push_system(format!(
                                "Unknown export format `{other}`. Use /export [md|json|html]"
                            ));
                            return true;
                        }
                    }
                };
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                let dir = std::path::Path::new(&home)
                    .join(".rs-agent")
                    .join("exports");
                let _ = std::fs::create_dir_all(&dir);
                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                let path = dir.join(format!("export_{}.{}", ts, fmt));
                let content = match SessionStore::new().load(&self.session_id) {
                    Ok(data) => match fmt {
                        "json" => match session::export_json(&data) {
                            Ok(s) => s,
                            Err(e) => {
                                self.push_system(format!("JSON export failed: {e}"));
                                return true;
                            }
                        },
                        "html" => session::export_html(&data),
                        _ => session::export_markdown(&data),
                    },
                    Err(_) => {
                        // Session not on disk yet — fall back to in-memory chat.
                        let mut md = format!("# rs-agent export {}\n\n", self.session_id);
                        for m in &self.messages {
                            md.push_str(&format!("## {}\n\n{}\n\n", m.role, m.text));
                            if let Some(ref th) = m.thinking {
                                if !th.is_empty() {
                                    md.push_str(&format!(
                                        "<details><summary>thinking</summary>\n\n{}\n\n</details>\n\n",
                                        th
                                    ));
                                }
                            }
                        }
                        if fmt == "json" {
                            serde_json::json!({"id": self.session_id, "fallback": true, "markdown": md})
                                .to_string()
                        } else if fmt == "html" {
                            format!(
                                "<!DOCTYPE html><html><body><pre>{}</pre></body></html>",
                                md.replace('<', "&lt;")
                            )
                        } else {
                            md
                        }
                    }
                };
                match std::fs::write(&path, content) {
                    Ok(()) => self.push_system(format!("Exported ({fmt}) to {}", path.display())),
                    Err(e) => self.push_system(format!("Export failed: {}", e)),
                }
            }
            "/trust" => {
                let mut parts = arg.splitn(2, char::is_whitespace);
                let sub = parts.next().unwrap_or("").trim();
                match sub {
                    "list" | "" => {
                        let paths = self.trust_store.list();
                        let cwd = std::env::current_dir()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let path_rules = self.path_allows.list_for_project(&cwd);
                        let mut out = String::new();
                        if paths.is_empty() {
                            out.push_str("No trusted projects.\n");
                        } else {
                            out.push_str("Trusted projects:\n");
                            for (p, trusted) in paths {
                                out.push_str(&format!(
                                    "  {} {}\n",
                                    if trusted { "✓" } else { "·" },
                                    p
                                ));
                            }
                        }
                        if path_rules.is_empty() {
                            out.push_str("No path-scoped allows for this project.");
                        } else {
                            out.push_str("Path allows (this project):\n");
                            for r in path_rules {
                                out.push_str(&format!("  {} → {}\n", r.tool, r.path_prefix));
                            }
                        }
                        self.push_system(out);
                    }
                    "reset" | "clear" => {
                        let cwd = std::env::current_dir()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        self.trust_store.clear();
                        self.path_allows.clear_all();
                        let _ = cwd;
                        self.push_system("Trust store and path allows cleared.");
                    }
                    _ => self.push_system("Usage: /trust list|reset"),
                }
            }
            "/compact" => {
                let _ = self.command_tx.send(AppCommand::Compact);
                self.status = "compacting...".to_string();
            }
            "/recall" => {
                let id = arg.trim();
                if id.is_empty() {
                    self.push_system(
                        "Usage: /recall <step id>  (ids are in the compaction index, e.g. s12)",
                    );
                } else {
                    match crate::agent::trajectory::recall(id) {
                        Ok(text) => self.push_system(text),
                        Err(e) => self.push_system(e),
                    }
                }
            }
            "/revert" => match crate::tools::turn_snapshot::restore_last_turn() {
                Ok(n) => {
                    self.status = format!("restored {n} file(s) from turn");
                    self.push_system(format!(
                        "Reverted last turn snapshot ({n} file(s) restored)."
                    ));
                }
                Err(e) => {
                    self.status = "revert failed".into();
                    self.push_system(format!("Revert failed: {e}"));
                }
            },
            "/new" => {
                self.start_new_project_session();
            }
            "/fork" => {
                let (at, label) = parse_fork_args(arg);
                let _ = self.command_tx.send(AppCommand::ForkSession { label, at });
                self.push_system(match at {
                    Some(n) => format!("Forking session at API message @{n}…"),
                    None => "Forking session…".into(),
                });
            }
            "/timeline" => {
                self.show_timeline_panel = !self.show_timeline_panel;
                if self.show_timeline_panel {
                    self.show_sessions_panel = false;
                    let _ = self.command_tx.send(AppCommand::RequestTimeline);
                    self.push_system(
                        "Timeline panel: j/k or ↑/↓ select · Enter fork at @N · Esc closes · /fork @N [label]",
                    );
                } else {
                    self.push_system("Timeline panel hidden.");
                }
            }
            "/lsp" => {
                let sub = arg.split_whitespace().next().unwrap_or("status");
                match sub {
                    "start" => {
                        if let Some(ref tx) = self.lsp_cmd_tx {
                            let _ = tx.send(LspCmd::Start);
                            self.push_system(
                                "Starting LSP (rust-analyzer, or RS_AGENT_LSP). Status appears in the footer.",
                            );
                        }
                    }
                    "stop" => {
                        if let Some(ref tx) = self.lsp_cmd_tx {
                            let _ = tx.send(LspCmd::Stop);
                        }
                        self.lsp_summary.clear();
                        self.push_system("LSP stopped.");
                    }
                    "status" | "" => {
                        if self.lsp_summary.is_empty() {
                            self.push_system(
                                "LSP idle. Use `/lsp start` (requires rust-analyzer on PATH).",
                            );
                        } else {
                            self.push_system(format!("LSP:{}", self.lsp_summary.trim()));
                        }
                    }
                    _ => self.push_system("Usage: /lsp start|stop|status"),
                }
            }
            "/model" => {
                if arg.is_empty() {
                    let cfg = crate::config::Config::load();
                    let mut names = crate::ai::registry::available_model_displays();
                    // Aliases as bare names (resolve against current provider).
                    for alias in cfg.model_aliases.keys() {
                        if !names.iter().any(|n| n == alias) {
                            names.push(alias.clone());
                        }
                    }
                    let current = format!("{}/{}", self.provider_name, self.model_name);
                    if !names.iter().any(|n| n == &current || n == &self.model_name) {
                        names.insert(0, current);
                    }
                    names.sort();
                    names.dedup();
                    let n = names.len();
                    self.picker_models = names;
                    self.model_cycle = self.picker_models.clone();
                    self.picker_prefix = String::new();
                    self.picker_query = String::new();
                    self.picker_mode = PickerMode::Model;
                    self.picker_selection = 0;
                    self.update_picker_results();
                    self.picker_active = true;
                    self.spawn_model_fetch();
                    let ready = crate::ai::registry::available_model_refs().len();
                    self.push_system(format!(
                        "Current: {}/{}\nCatalog picker: {} models from {} ready provider(s).\n\
                         (Only providers with API keys are listed — export OPENROUTER_API_KEY etc. for more.\n\
                         Live fetch merging in…)",
                        self.provider_name, self.model_name, n, ready
                    ));
                } else {
                    match self.apply_model_selection(arg) {
                        Ok(msg) => self.push_system(msg),
                        Err(e) => self.push_system(e),
                    }
                }
            }
            "/provider" | "/login" => {
                if arg.is_empty() {
                    self.start_provider_picker();
                } else if arg.eq_ignore_ascii_case("list") {
                    let mut lines = vec![
                        format!("Current provider: {}", self.provider_name),
                        "Providers:".to_string(),
                    ];
                    lines.extend(crate::ai::registry::provider_status_lines());
                    self.push_system(lines.join("\n"));
                } else {
                    self.select_or_connect_provider(&arg.to_lowercase());
                }
            }
            "/tree" => {
                self.show_timeline_panel = false;
                if self.show_tree_panel && self.side_mode == SidePanelMode::Tree {
                    self.show_tree_panel = false;
                } else {
                    self.show_tree_panel = true;
                    self.side_mode = SidePanelMode::Tree;
                }
                let panel_note = if self.show_tree_panel {
                    "shown"
                } else {
                    "hidden"
                };
                if self.tree_breadcrumb != "idle" {
                    self.push_system(format!(
                        "Call tree panel {}.\nCall tree: {}\n(depth max {})",
                        panel_note, self.tree_breadcrumb, self.rlm_depth
                    ));
                } else {
                    let saved_summary = SessionStore::new()
                        .load(&self.session_id)
                        .ok()
                        .and_then(|data| data.call_tree)
                        .and_then(|v| {
                            serde_json::from_value::<crate::rlm::tree::CallTreeInner>(v).ok()
                        })
                        .map(|inner| inner.summary());
                    match saved_summary {
                        Some(summary) => {
                            self.tree_panel_text = summary.clone();
                            self.push_system(format!(
                                "Call tree panel {}.\nCall tree: idle (no active run)\nLast saved snapshot — {}\n(depth max {})",
                                panel_note, summary, self.rlm_depth
                            ))
                        }
                        None => self.push_system(format!(
                            "Call tree panel {}.\nCall tree: {}\n(depth max {})",
                            panel_note, self.tree_breadcrumb, self.rlm_depth
                        )),
                    }
                }
            }
            "/output" => {
                self.show_repl_panel = !self.show_repl_panel;
                let n = self.tool_output_tabs.len();
                self.push_system(format!(
                    "Bottom console {} — {n} run(s). Tab / ↑↓ switch while waiting.",
                    if self.show_repl_panel {
                        "shown"
                    } else {
                        "hidden"
                    }
                ));
            }
            _ => {
                self.push_system(format!("Unknown command: {} (try /help)", cmd));
            }
        }
        true
    }

    /// Push a non-slash user message to the agent: records it in
    /// `input_history`, appends chat bubbles, and dispatches `Submit`.
    pub(super) fn submit_user_text(&mut self, text: String) {
        if self.input_history.last().map(|s| s.as_str()) != Some(text.as_str()) {
            self.input_history.push(text.clone());
        }
        self.history_index = None;

        let expanded = Self::expand_dir_tokens(&text);

        self.input_mode = InputMode::Waiting;
        self.agent_turn_active = true;
        if let Some(rt) = self.runtimes.get_mut(&self.session_id) {
            rt.turn_active = true;
        }
        self.queued_steers = 0;
        if self.tool_in_progress.is_none() {
            self.tool_output_tabs.clear();
            self.tool_output_tab = 0;
            self.repl_panel.clear();
        }

        self.messages.push(ChatMessage {
            role: "user".to_string(),
            text: expanded.clone(),
            thinking: None,
            show_thinking: false,
            tool_blocks: Vec::new(),
        });
        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            text: String::new(),
            thinking: None,
            show_thinking: false,
            tool_blocks: Vec::new(),
        });

        self.follow_bottom = true;
        self.unseen_done = false;
        self.status = "thinking...".to_string();
        let _ = self.command_tx.send(AppCommand::Submit { text: expanded });
    }

    /// Expands any `#somedir` tokens in `text` into a short, non-recursive
    /// directory listing wrapped in a `<dir>` block, so the agent gets a
    /// quick summary without a separate tool call. Tokens that aren't
    /// readable directories are left untouched.
    fn expand_dir_tokens(text: &str) -> String {
        let re = regex::Regex::new(r"#(\S+)").expect("static regex is valid");
        re.replace_all(text, |caps: &regex::Captures| {
            let path = &caps[1];
            Self::summarize_dir(path).unwrap_or_else(|| format!("#{}", path))
        })
        .to_string()
    }

    /// Reads up to `MAX_ENTRIES` non-recursive directory entries at `path`
    /// and renders them as a `<dir path="...">...</dir>` block. Returns
    /// `None` if `path` isn't a readable directory.
    fn summarize_dir(path: &str) -> Option<String> {
        const MAX_ENTRIES: usize = 40;
        let read_dir = std::fs::read_dir(path).ok()?;
        let mut names: Vec<String> = read_dir
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    format!("{}/", name)
                } else {
                    name
                }
            })
            .collect();
        names.sort();
        let truncated = names.len() > MAX_ENTRIES;
        names.truncate(MAX_ENTRIES);

        let mut out = format!("<dir path=\"{}\">\n", path);
        for name in &names {
            out.push_str(name);
            out.push('\n');
        }
        if truncated {
            out.push_str("...\n");
        }
        out.push_str("</dir>");
        Some(out)
    }

    /// Rebuilds the system prompt from scratch (default prompt + optional
    /// project context sections), used by `/reload` and `/context on|off`.
    /// Custom `--system-prompt` / `--append-system-prompt` CLI overrides are
    /// intentionally not re-applied here — a full rebuild only restores the
    /// default prompt plus whatever context toggles are currently active.
    fn rebuild_system_prompt(&self) -> String {
        let mut sp = crate::agent::default_system_prompt();
        if self.context_enabled {
            let files = discover_context_files();
            sp.push_str(&build_context_section(&files));
            let cmds = discover_agent_commands();
            sp.push_str(&build_commands_section(&cmds));
        }
        sp
    }
}
