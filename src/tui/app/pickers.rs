//! Interactive pickers — files, directories, skills, models, providers —
//! plus fuzzy ranking and Tab completion.
//!
use super::*;

impl App {
    /// Kicks off a background fetch of models from **all** providers with
    /// configured auth; results are `provider/model` strings.
    pub(super) fn spawn_model_fetch(&mut self) {
        let (tx, rx) = channel::unbounded::<Vec<String>>();
        self.model_picker_rx = Some(rx);
        let timeout = self.timeout_secs;
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(_) => return,
            };
            let list = rt.block_on(crate::ai::registry::fetch_all_model_displays(timeout));
            let _ = tx.send(list);
        });
    }

    /// Apply a model selection string: alias, bare model, or `provider/model`.
    pub(super) fn apply_model_selection(&mut self, raw: &str) -> Result<String, String> {
        let cfg = crate::config::Config::load();
        let resolved_arg = cfg.resolve_model_alias(raw);
        let mref = crate::ai::registry::ModelRef::parse(&resolved_arg, &self.provider_name);

        if !crate::ai::registry::is_known_provider(&mref.provider) {
            return Err(format!("Unknown provider `{}`", mref.provider));
        }
        // Prefer OpenCode Zen keys from the local OpenCode install when unset.
        if mref.provider.eq_ignore_ascii_case("opencode")
            || mref.provider.eq_ignore_ascii_case("opencode-go")
        {
            crate::ai::registry::export_opencode_auth_from_file();
        }
        if !crate::ai::registry::has_configured_auth(&mref.provider) {
            return Err(format!(
                "Provider `{}` has no credentials (export {} or sign in via OpenCode).",
                mref.provider,
                crate::ai::registry::api_key_env_for(&mref.provider)
            ));
        }

        let same_provider = mref.provider.eq_ignore_ascii_case(&self.provider_name);
        let needs_recreate = crate::ai::registry::provider_client_needs_recreate(
            &self.provider_name,
            &self.model_name,
            &mref.provider,
            &mref.model,
        );
        if same_provider && !needs_recreate {
            self.model_name = mref.model.clone();
            let _ = self.command_tx.send(AppCommand::SetModel {
                model: mref.model.clone(),
            });
            self.token_limit = crate::ai::token_count::get_context_limit(&mref.model);
            self.note_cycle_entry(&mref.display());
            Self::remember_selection(&mref.provider, &mref.model);
            let mut msg = if resolved_arg != raw {
                format!(
                    "Model set to {}/{} (alias `{}`)",
                    mref.provider, mref.model, raw
                )
            } else {
                format!("Model set to {}/{}", mref.provider, mref.model)
            };
            if let Some(w) = crate::agent::weak_model_user_warning(&mref.model) {
                msg.push('\n');
                msg.push_str(&w);
            }
            return Ok(msg);
        }

        let provider = crate::ai::registry::create_provider(
            &mref.provider,
            crate::ai::registry::CreateProviderOpts {
                default_model: Some(mref.model.clone()),
                timeout_secs: self.timeout_secs,
                ..Default::default()
            },
        )?;
        self.provider = provider.clone();
        self.provider_name = mref.provider.clone();
        self.model_name = mref.model.clone();
        self.token_limit = crate::ai::token_count::get_context_limit(&mref.model);
        let _ = self.command_tx.send(AppCommand::SetProvider {
            provider,
            model: mref.model.clone(),
        });
        self.note_cycle_entry(&mref.display());
        Self::remember_selection(&mref.provider, &mref.model);
        let kind = if same_provider {
            "model + API endpoint"
        } else {
            "provider + model"
        };
        Ok(format!(
            "Switched to {}/{} ({})",
            mref.provider, mref.model, kind
        ))
    }

    /// Best-effort write last provider/model to ~/.rs-agent/config.toml.
    fn remember_selection(provider: &str, model: &str) {
        if let Err(e) = crate::config::Config::persist_last_selection(provider, model) {
            eprintln!("Warning: could not remember provider/model: {e}");
        }
    }

    fn note_cycle_entry(&mut self, display: &str) {
        if !self.model_cycle.iter().any(|c| c == display) {
            self.model_cycle.push(display.to_string());
        }
        if let Some(i) = self.model_cycle.iter().position(|c| c == display) {
            self.model_cycle_index = i;
        }
    }

    /// Cycle to the next ready provider/model (pi Ctrl+P).
    pub(super) fn cycle_model(&mut self) {
        if self.model_cycle.is_empty() {
            self.model_cycle = crate::ai::registry::available_model_refs()
                .into_iter()
                .map(|r| r.display())
                .collect();
        }
        if self.model_cycle.is_empty() {
            self.push_system("No providers with credentials to cycle.");
            return;
        }
        self.model_cycle_index = (self.model_cycle_index + 1) % self.model_cycle.len();
        let next = self.model_cycle[self.model_cycle_index].clone();
        match self.apply_model_selection(&next) {
            Ok(msg) => self.push_system(format!("Cycled — {}", msg)),
            Err(e) => self.push_system(e),
        }
    }

    /// Drains a completed model fetch (if any) into `picker_models`, and
    /// refreshes the active picker's results if it's currently showing models.
    pub(super) fn poll_model_picker(&mut self) {
        let Some(rx) = &self.model_picker_rx else {
            return;
        };
        let Ok(list) = rx.try_recv() else {
            return;
        };
        for m in list {
            if !self.picker_models.contains(&m) {
                self.picker_models.push(m.clone());
            }
            if !self.model_cycle.contains(&m) {
                self.model_cycle.push(m);
            }
        }
        self.picker_models.sort();
        if self.picker_active && self.picker_mode == PickerMode::Model {
            self.update_picker_results();
        }
        self.model_picker_rx = None;
    }

    /// If the input is `/skill`, `/prompt`, `/model`, `/theme`, `/mode`, or
    /// `/provider` with a partial arg, either completes the single match or
    /// opens a picker. No-op otherwise.
    pub(super) fn try_start_completion_picker(&mut self) {
        let text = self.input.clone();

        // Fixed-choice slash commands
        let fixed: &[(&str, &[&str], PickerMode)] = &[
            ("/theme ", &["dark", "light", "forest"], PickerMode::Prompt),
            ("/mode ", &["plan", "ask", "agent"], PickerMode::Prompt),
        ];
        for (cmd, options, mode) in fixed {
            let Some(rest) = text.strip_prefix(cmd) else {
                continue;
            };
            if rest.contains(char::is_whitespace) {
                continue;
            }
            let query_lower = rest.to_lowercase();
            let names: Vec<String> = options.iter().map(|s| (*s).to_string()).collect();
            let filtered: Vec<String> = if query_lower.is_empty() {
                names
            } else {
                Self::rank_and_filter(&names, &query_lower, 20)
            };
            match filtered.len() {
                0 => {}
                1 => self.input = format!("{}{} ", cmd, filtered[0]),
                _ => {
                    self.picker_prefix = cmd.to_string();
                    self.picker_query = rest.to_string();
                    self.picker_mode = *mode;
                    self.picker_results = filtered;
                    self.picker_selection = 0;
                    self.picker_active = true;
                }
            }
            return;
        }

        // /model — use available model displays
        if let Some(rest) = text.strip_prefix("/model ") {
            if !rest.contains(char::is_whitespace) {
                let mut names = crate::ai::registry::available_model_displays();
                let cfg = crate::config::Config::load();
                for alias in cfg.model_aliases.keys() {
                    if !names.iter().any(|n| n == alias) {
                        names.push(alias.clone());
                    }
                }
                let query_lower = rest.to_lowercase();
                let filtered: Vec<String> = if query_lower.is_empty() {
                    names
                } else {
                    Self::rank_and_filter(&names, &query_lower, 50)
                };
                match filtered.len() {
                    0 => {}
                    1 => self.input = format!("/model {} ", filtered[0]),
                    _ => {
                        self.picker_prefix = "/model ".into();
                        self.picker_query = rest.to_string();
                        self.picker_models = filtered.clone();
                        self.picker_mode = PickerMode::Model;
                        self.picker_results = filtered;
                        self.picker_selection = 0;
                        self.picker_active = true;
                    }
                }
                return;
            }
        }

        // /provider|/login
        for cmd in ["/provider ", "/login "] {
            let Some(rest) = text.strip_prefix(cmd) else {
                continue;
            };
            if rest.contains(char::is_whitespace) {
                continue;
            }
            let names: Vec<String> = crate::ai::registry::provider_picker_rows()
                .into_iter()
                .filter_map(|row| row.split_whitespace().next().map(|s| s.to_string()))
                .collect();
            let query_lower = rest.to_lowercase();
            let filtered: Vec<String> = if query_lower.is_empty() {
                names
            } else {
                Self::rank_and_filter(&names, &query_lower, 40)
            };
            match filtered.len() {
                0 => {}
                1 => self.input = format!("{cmd}{} ", filtered[0]),
                _ => {
                    self.picker_prefix = cmd.to_string();
                    self.picker_query = rest.to_string();
                    self.picker_mode = PickerMode::Provider;
                    self.picker_results = filtered;
                    self.picker_selection = 0;
                    self.picker_active = true;
                }
            }
            return;
        }

        let candidates: [(&str, bool); 3] =
            [("/skill ", true), ("/prompt ", false), ("/p ", false)];
        for (cmd, is_skill) in candidates {
            let Some(rest) = text.strip_prefix(cmd) else {
                continue;
            };
            if rest.contains(char::is_whitespace) {
                continue;
            }
            let prefix = cmd.to_string();
            let query = rest.to_string();
            let names: Vec<String> = if is_skill {
                discover_skills().into_iter().map(|s| s.name).collect()
            } else {
                discover_templates().into_iter().map(|t| t.name).collect()
            };
            let query_lower = query.to_lowercase();
            let filtered: Vec<String> = if query_lower.is_empty() {
                names
            } else {
                Self::rank_and_filter(&names, &query_lower, 50)
            };
            match filtered.len() {
                0 => {}
                1 => {
                    self.input = format!("{}{} ", prefix, filtered[0]);
                }
                _ => {
                    self.picker_prefix = prefix;
                    self.picker_query = query;
                    self.picker_mode = if is_skill {
                        PickerMode::Skill
                    } else {
                        PickerMode::Prompt
                    };
                    self.picker_results = filtered;
                    self.picker_selection = 0;
                    self.picker_active = true;
                }
            }
            return;
        }
    }

    const PICKER_SKIP_DIRS: [&'static str; 4] = ["target", "node_modules", ".git", "__pycache__"];

    const PICKER_RESULT_CAP: usize = 50;

    pub(super) fn start_picker(&mut self) {
        self.picker_prefix = self.input.clone();
        self.picker_query = String::new();
        self.picker_selection = 0;
        self.picker_mode = PickerMode::File;
        // Always rescan — cwd may have changed / first open may have raced.
        self.picker_files_loaded = false;
        self.picker_active = true;
        self.update_picker_results();
        self.status = format!(
            "file picker: {} file(s) — type to filter, Enter to insert path",
            self.picker_files.len()
        );
    }

    pub(super) fn start_dir_picker(&mut self) {
        self.picker_prefix = self.input.clone();
        self.picker_query = String::new();
        self.picker_selection = 0;
        self.picker_mode = PickerMode::Dir;
        self.picker_dirs_loaded = false;
        self.picker_active = true;
        self.update_picker_results();
        self.status = format!(
            "dir picker: {} dir(s) — type to filter, Enter to attach #path",
            self.picker_dirs.len()
        );
    }

    pub(super) fn start_provider_picker(&mut self) {
        self.picker_prefix = String::new();
        self.picker_query = String::new();
        self.picker_selection = 0;
        self.picker_providers = crate::ai::registry::provider_picker_rows();
        // Put current provider near the top if present.
        if let Some(idx) = self
            .picker_providers
            .iter()
            .position(|r| r.starts_with(&format!("{} ", self.provider_name)))
        {
            let cur = self.picker_providers.remove(idx);
            self.picker_providers.insert(0, cur);
        }
        self.picker_mode = PickerMode::Provider;
        self.picker_active = true;
        self.input_mode = InputMode::Insert;
        self.update_picker_results();
        self.push_system(format!(
            "Provider picker — current: {}.\n\
             Select a [ready] provider to switch, or a [needs key] row to open its signup page and paste an API key.\n\
             Keys are saved to ~/.rs-agent/secrets.toml",
            self.provider_name
        ));
        self.status = format!(
            "provider picker: {} — Enter to select / connect",
            self.picker_providers.len()
        );
    }

    /// Switch to a ready provider, or start the connect/key-paste flow.
    pub(super) fn select_or_connect_provider(&mut self, pname: &str) {
        if !crate::ai::registry::is_known_provider(pname)
            && !crate::ai::registry::supports_runtime(pname)
        {
            self.push_system(format!("Unknown provider `{pname}`"));
            return;
        }
        if !crate::ai::registry::supports_runtime(pname) {
            self.push_system(format!(
                "Provider `{pname}` is in the catalog but not runnable yet."
            ));
            return;
        }
        // Reload secrets into env in case they were just written.
        crate::config::export_secrets_to_env();
        if crate::ai::registry::has_configured_auth(pname) {
            let model = if self.provider_name.eq_ignore_ascii_case(pname) {
                self.model_name.clone()
            } else {
                crate::ai::registry::default_model_for(pname)
            };
            match self.apply_model_selection(&format!("{pname}/{model}")) {
                Ok(msg) => self.push_system(msg),
                Err(e) => self.push_system(e),
            }
            return;
        }
        // Needs a key: open browser + enter capture mode.
        let env = crate::ai::registry::api_key_env_for(pname);
        match crate::ai::registry::open_provider_connect_url(pname) {
            Ok(url) => self.push_system(format!(
                "Opening {url}\nCreate an API key, then paste it below and press Enter.\n\
                 (Or Esc to cancel. Stored as {env} in ~/.rs-agent/secrets.toml)"
            )),
            Err(e) => self.push_system(format!(
                "{e}\nPaste your API key for `{pname}` ({env}) and press Enter.\nEsc to cancel."
            )),
        }
        self.pending_api_key_provider = Some(pname.to_string());
        self.input.clear();
        self.input_mode = InputMode::ApiKey;
        self.picker_active = false;
        self.status = format!("paste API key for {pname} · Enter save · Esc cancel");
    }

    pub(super) fn handle_api_key_input(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.pending_api_key_provider = None;
                self.input.clear();
                self.input_mode = InputMode::Insert;
                self.status = "cancelled API key entry".to_string();
            }
            KeyCode::Enter => {
                let Some(pname) = self.pending_api_key_provider.take() else {
                    self.input_mode = InputMode::Insert;
                    return;
                };
                let key_text = std::mem::take(&mut self.input);
                match crate::config::store_api_key(&pname, &key_text) {
                    Ok(()) => {
                        self.input_mode = InputMode::Insert;
                        self.push_system(format!(
                            "Saved API key for `{pname}` → {} (also exported to env).",
                            crate::ai::registry::api_key_env_for(&pname)
                        ));
                        let model = crate::ai::registry::default_model_for(&pname);
                        match self.apply_model_selection(&format!("{pname}/{model}")) {
                            Ok(msg) => {
                                self.push_system(msg);
                                self.status = format!("connected {pname}");
                            }
                            Err(e) => {
                                self.push_system(format!(
                                    "Key saved, but switch failed: {e}. Try /model."
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        self.pending_api_key_provider = Some(pname);
                        self.push_system(format!("Could not save key: {e}"));
                        self.input_mode = InputMode::ApiKey;
                    }
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            _ => {}
        }
    }

    pub(super) fn update_picker_results(&mut self) {
        match self.picker_mode {
            PickerMode::File => {
                if !self.picker_files_loaded {
                    self.load_picker_files();
                }
                self.picker_results = Self::rank_and_filter(
                    &self.picker_files,
                    &self.picker_query,
                    Self::PICKER_RESULT_CAP,
                );
            }
            PickerMode::Dir => {
                if !self.picker_dirs_loaded {
                    self.load_picker_dirs();
                }
                self.picker_results = Self::rank_and_filter(
                    &self.picker_dirs,
                    &self.picker_query,
                    Self::PICKER_RESULT_CAP,
                );
            }
            PickerMode::Skill => {
                let names: Vec<String> = discover_skills().into_iter().map(|s| s.name).collect();
                self.picker_results =
                    Self::rank_and_filter(&names, &self.picker_query, Self::PICKER_RESULT_CAP);
            }
            PickerMode::Prompt => {
                let names: Vec<String> = discover_templates().into_iter().map(|t| t.name).collect();
                self.picker_results =
                    Self::rank_and_filter(&names, &self.picker_query, Self::PICKER_RESULT_CAP);
            }
            PickerMode::Model => {
                self.picker_results = Self::rank_and_filter(
                    &self.picker_models,
                    &self.picker_query,
                    Self::PICKER_RESULT_CAP,
                );
            }
            PickerMode::Provider => {
                self.picker_results = Self::rank_and_filter(
                    &self.picker_providers,
                    &self.picker_query,
                    Self::PICKER_RESULT_CAP,
                );
            }
        }
        self.picker_selection = self
            .picker_selection
            .min(self.picker_results.len().saturating_sub(1));
    }

    /// Ranks `items` against `query` with fuzzy (subsequence) matching.
    /// Spaces split the query into tokens that must all match. Lower score
    /// is better: exact / prefix / contiguous / fuzzy. Caps at `cap`.
    pub(super) fn rank_and_filter(items: &[String], query: &str, cap: usize) -> Vec<String> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            let mut out: Vec<String> = items.to_vec();
            out.truncate(cap);
            return out;
        }
        let tokens: Vec<&str> = query.split_whitespace().filter(|t| !t.is_empty()).collect();
        let mut scored: Vec<(i32, usize, &String)> = items
            .iter()
            .filter_map(|item| {
                let lower = item.to_lowercase();
                let mut total = 0i32;
                for tok in &tokens {
                    let s = Self::fuzzy_score(&lower, tok)?;
                    total += s;
                }
                Some((total, item.len(), item))
            })
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(b.2)));
        scored
            .into_iter()
            .take(cap)
            .map(|(_, _, s)| s.clone())
            .collect()
    }

    /// Fuzzy score for `haystack` against `needle` (both already lowercased).
    /// `None` if needle chars are not a subsequence of haystack.
    /// Lower is better.
    pub(super) fn fuzzy_score(haystack: &str, needle: &str) -> Option<i32> {
        if needle.is_empty() {
            return Some(0);
        }
        if haystack == needle {
            return Some(0);
        }
        // Prefer matches on the final path segment (model id).
        let base = haystack.rsplit('/').next().unwrap_or(haystack);
        if let Some(s) = Self::fuzzy_score_one(base, needle) {
            return Some(s);
        }
        Self::fuzzy_score_one(haystack, needle).map(|s| s + 20)
    }

    pub(super) fn fuzzy_score_one(haystack: &str, needle: &str) -> Option<i32> {
        if haystack == needle {
            return Some(0);
        }
        if haystack.starts_with(needle) {
            return Some(1);
        }
        if let Some(pos) = haystack.find(needle) {
            // Contiguous substring: earlier + word-ish boundary is better.
            let mut score = 10 + pos as i32;
            if pos > 0 {
                let prev = haystack.as_bytes()[pos - 1] as char;
                if matches!(prev, '/' | '-' | '_' | '.' | ' ') {
                    score -= 5;
                }
            }
            return Some(score);
        }

        // Subsequence: every needle char appears in order in haystack.
        let h: Vec<char> = haystack.chars().collect();
        let n: Vec<char> = needle.chars().collect();
        let mut hi = 0usize;
        let mut score = 40i32;
        let mut prev_match = None::<usize>;
        let mut gaps = 0i32;
        for &nc in &n {
            let mut found = false;
            while hi < h.len() {
                if h[hi] == nc {
                    if let Some(p) = prev_match {
                        let gap = (hi - p) as i32 - 1;
                        gaps += gap;
                        // Bonus for consecutive matches.
                        if gap == 0 {
                            score -= 2;
                        }
                    } else if hi == 0
                        || matches!(h[hi.saturating_sub(1)], '/' | '-' | '_' | '.' | ' ')
                    {
                        score -= 3; // start-of-token bonus
                    }
                    prev_match = Some(hi);
                    hi += 1;
                    found = true;
                    break;
                }
                hi += 1;
            }
            if !found {
                return None;
            }
        }
        Some(score + gaps)
    }

    /// Builds a rough gitignore-style [`GlobSet`] from a hardcoded list of
    /// common noisy directories plus a best-effort parse of `.gitignore` at
    /// `cwd` (negations and complex patterns are not supported).
    fn build_ignore_globset(cwd: &std::path::Path) -> GlobSet {
        let mut builder = GlobSetBuilder::new();
        for pat in [
            "**/target/**",
            "**/node_modules/**",
            "**/.git/**",
            "**/__pycache__/**",
            "**/*.o",
        ] {
            if let Ok(g) = Glob::new(pat) {
                builder.add(g);
            }
        }
        if let Ok(content) = std::fs::read_to_string(cwd.join(".gitignore")) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                    continue;
                }
                let mut pat = line.trim_end_matches('/').to_string();
                if let Some(stripped) = pat.strip_prefix('/') {
                    pat = stripped.to_string();
                } else if !pat.contains('/') {
                    pat = format!("**/{}", pat);
                }
                for variant in [pat.clone(), format!("{}/**", pat)] {
                    if let Ok(g) = Glob::new(&variant) {
                        builder.add(g);
                    }
                }
            }
        }
        builder
            .build()
            .unwrap_or_else(|_| GlobSetBuilder::new().build().unwrap())
    }

    fn picker_filter_entry(entry: &walkdir::DirEntry) -> bool {
        let name = entry.file_name().to_string_lossy();
        if entry.depth() > 0 && name.starts_with('.') {
            return false;
        }
        if entry.file_type().is_dir() && Self::PICKER_SKIP_DIRS.contains(&name.as_ref()) {
            return false;
        }
        true
    }

    fn load_picker_files(&mut self) {
        self.picker_files.clear();
        let cwd = std::env::current_dir().unwrap_or_default();
        let ignore_set = Self::build_ignore_globset(&cwd);
        const MAX_FILES: usize = 8_000;
        const MAX_DEPTH: usize = 12;
        for entry in WalkDir::new(&cwd)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_entry(Self::picker_filter_entry)
        {
            if self.picker_files.len() >= MAX_FILES {
                break;
            }
            if let Ok(entry) = entry {
                if entry.file_type().is_file() {
                    if let Ok(relative) = entry.path().strip_prefix(&cwd) {
                        let path = relative.to_string_lossy().replace('\\', "/");
                        if path.is_empty() || ignore_set.is_match(path.as_str()) {
                            continue;
                        }
                        self.picker_files.push(path);
                    }
                }
            }
        }
        self.picker_files.sort();
        self.picker_files_loaded = true;
    }

    fn load_picker_dirs(&mut self) {
        self.picker_dirs.clear();
        let cwd = std::env::current_dir().unwrap_or_default();
        let ignore_set = Self::build_ignore_globset(&cwd);
        const MAX_DIRS: usize = 4_000;
        const MAX_DEPTH: usize = 12;
        for entry in WalkDir::new(&cwd)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_entry(Self::picker_filter_entry)
        {
            if self.picker_dirs.len() >= MAX_DIRS {
                break;
            }
            if let Ok(entry) = entry {
                if entry.file_type().is_dir() && entry.depth() > 0 {
                    if let Ok(relative) = entry.path().strip_prefix(&cwd) {
                        let path = relative.to_string_lossy().replace('\\', "/");
                        if path.is_empty()
                            || ignore_set.is_match(path.as_str())
                            || ignore_set.is_match(format!("{}/", path).as_str())
                        {
                            continue;
                        }
                        self.picker_dirs.push(path);
                    }
                }
            }
        }
        self.picker_dirs.sort();
        self.picker_dirs_loaded = true;
    }

    pub(super) fn render_picker(&mut self, frame: &mut Frame, area: Rect) {
        let empty = self.picker_results.is_empty();
        let picker_height = if empty {
            1
        } else {
            (self.picker_results.len() as u16).clamp(1, 10)
        };
        let width = if self.picker_mode == PickerMode::Provider {
            96
        } else {
            72
        }
        .min(area.width.saturating_sub(2));
        let height = picker_height + 2;
        // Anchor near input (bottom) like a command palette
        let picker_area = Rect {
            x: area.x + 1,
            y: area
                .y
                .saturating_add(area.height.saturating_sub(height + 4)),
            width,
            height,
        };

        let items: Vec<ListItem> = if empty {
            let hint = match self.picker_mode {
                PickerMode::File => "(no files found — check cwd / .gitignore)",
                PickerMode::Dir => "(no directories found)",
                PickerMode::Model => "(no models — export API keys, or wait for fetch)",
                PickerMode::Provider => "(no providers — catalog empty?)",
                PickerMode::Skill => "(no skills found)",
                PickerMode::Prompt => "(no templates found)",
            };
            vec![ListItem::new(hint).style(Style::default().fg(self.palette.muted))]
        } else {
            self.picker_results
                .iter()
                .enumerate()
                .map(|(i, path)| {
                    let style = if i == self.picker_selection {
                        Style::default()
                            .fg(self.palette.highlight_fg)
                            .bg(self.palette.highlight_bg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(self.palette.subtext)
                    };
                    ListItem::new(path.as_str()).style(style)
                })
                .collect()
        };

        let title = match self.picker_mode {
            PickerMode::File => "Files · Enter · Esc",
            PickerMode::Dir => "Directories · Enter · Esc",
            PickerMode::Skill => "Skills",
            PickerMode::Prompt => "Templates",
            PickerMode::Model => "Models",
            PickerMode::Provider => "Providers · Enter connect · Esc",
        };
        use ratatui::widgets::Clear;
        frame.render_widget(Clear, picker_area);
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} "))
                .border_style(
                    Style::default()
                        .fg(self.palette.accent)
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(self.palette.panel_bg)),
        );

        frame.render_widget(list, picker_area);
    }
}
