use crate::agent::r#loop::AgentEvent;
use crate::agent::state::AgentState;
use crate::agent::AgentLoop;
use crate::ai::provider::Provider;
use crate::ai::types::Message;
use crate::permission::{PendingPermission, PermissionReply, TrustStore};
use crate::session::{SessionData, SessionStore};
use crossbeam_channel as channel;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::renderer::render_markdown;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use walkdir::WalkDir;

#[derive(Clone)]
struct ChatMessage {
    role: String,
    text: String,
}

#[derive(PartialEq)]
enum InputMode {
    Normal,
    Insert,
    Waiting,
}

#[allow(dead_code)]
enum AppCommand {
    Submit { text: String },
    Init { messages: Vec<Message> },
    Exit,
}

pub struct App {
    messages: Vec<ChatMessage>,
    input: String,
    input_mode: InputMode,
    should_exit: bool,
    status: String,
    command_tx: channel::Sender<AppCommand>,
    event_rx: channel::Receiver<(usize, AgentEvent)>,
    scroll_offset: usize,
    follow_bottom: bool,
    picker_active: bool,
    picker_prefix: String,
    picker_query: String,
    picker_results: Vec<String>,
    picker_selection: usize,
    picker_files: Vec<String>,
    picker_files_loaded: bool,
    pending_permission: Option<PendingPermission>,
    permission_rx: channel::Receiver<PendingPermission>,
    trust_store: TrustStore,
    #[allow(dead_code)]
    approved: bool,
    token_used: usize,
    token_limit: usize,
    near_limit: bool,
    session_id: String,
}

impl App {
    pub fn new(provider: Arc<dyn Provider>, model: String, timeout_secs: u64, approve: bool, resume: Option<SessionData>, system_prompt: Option<String>) -> Self {
        let (command_tx, command_rx) = channel::unbounded::<AppCommand>();
        let (event_tx, event_rx) = channel::unbounded::<(usize, AgentEvent)>();
        let (permission_tx, permission_rx) = channel::unbounded::<PendingPermission>();

        let provider_name = provider.name().to_string();
        let provider2 = provider.clone();
        let model2 = model.clone();
        let timeout = std::time::Duration::from_secs(timeout_secs);
        let session_id =
            resume.as_ref().map(|s| s.id.clone()).unwrap_or_else(SessionStore::generate_id);
        let created_at = resume.as_ref().map(|s| s.created_at.clone()).unwrap_or_else(|| {
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
        });
        let resume_msgs = resume.as_ref().map(|s| s.messages.clone()).unwrap_or_default();
        let session_id_for_thread = session_id.clone();
        let created_at_for_thread = created_at.clone();
        let system_prompt_for_thread = system_prompt.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let sp = system_prompt_for_thread.unwrap_or_else(|| {
                    "You are an expert coding assistant operating inside rs-agent, a coding agent harness. \
                     You help users by reading files, executing commands, editing code, and writing new files.\n\n\
                     Guidelines:\n\
                     - Use `read` to examine files instead of cat or sed. For text files, read shows content with line numbers.\n\
                     - Use `bash` to execute commands. Prefer using bash over read for file listing (ls, find).\n\
                     - Use `edit` for precise changes to existing files. Provide exact oldText to match.\n\
                     - Use `write` to create new files or complete rewrites.\n\
                     - Use `grep` to search for patterns in the codebase.\n\
                     - Use `ls` to list directory contents.\n\
                     - When writing code, first understand the existing patterns, then implement, then test.\n\
                     - Always check if the code compiles/runs correctly after making changes."
                        .to_string()
                });

                let mut state = AgentState::new(model2, provider_name)
                    .with_system_prompt(sp);

                for msg in &resume_msgs {
                    state.add_message(msg.clone());
                }

                let mut agent_loop = AgentLoop::new(provider2, state);
                if !approve {
                    agent_loop.set_permission_channel(permission_tx);
                }
                crate::tools::register_default_tools(&mut agent_loop);

                let store = SessionStore::new();

                loop {
                    let cmd = command_rx.recv().unwrap_or(AppCommand::Exit);
                    match cmd {
                        AppCommand::Exit => break,
                        AppCommand::Init { messages } => {
                            for msg in messages {
                                agent_loop.state_mut().add_message(msg);
                            }
                        }
                        AppCommand::Submit { text } => {
                            let result = tokio::time::timeout(
                                timeout,
                                agent_loop.run(&text, &mut |event: AgentEvent| {
                                    let _ = event_tx.send((0, event));
                                }),
                            )
                            .await;
                            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                            match result {
                                Ok(Ok(())) => {}
                                Ok(Err(e)) => {
                                    let _ = event_tx.send((0, AgentEvent::Error { message: e }));
                                }
                                Err(_) => {
                                    let _ = event_tx.send((0, AgentEvent::Error {
                                        message: "Request timed out after 120s".to_string(),
                                    }));
                                }
                            }
                            let s = agent_loop.state();
                            let _ = store.save(&SessionData {
                                id: session_id_for_thread.clone(),
                                created_at: created_at_for_thread.clone(),
                                updated_at: now,
                                model: s.model.clone(),
                                provider: s.provider.clone(),
                                system_prompt: s.system_prompt.clone(),
                                messages: s.messages.clone(),
                                total_input_tokens: s.total_input_tokens,
                                total_output_tokens: s.total_output_tokens,
                            });
                        }
                    }
                }
            });
        });

        let trust_store = TrustStore::new();

        let mut initial_msgs = vec![ChatMessage {
            role: "system".to_string(),
            text: format!(
                "Rs Agent - Minimalist AI Agent Toolkit\nModel: {}\nSession: {}\n\nType a message to start a conversation.\ni: insert mode | Esc: normal mode | ^C: quit",
                model, session_id
            ),
        }];

        if let Some(ref resume_data) = resume {
            for msg in &resume_data.messages {
                let (role, text) = match &msg.role {
                    crate::ai::types::Role::User => ("user", msg.content.first().and_then(|c| c.text.as_deref()).unwrap_or("")),
                    crate::ai::types::Role::Assistant => ("assistant", msg.content.first().and_then(|c| c.text.as_deref()).unwrap_or("")),
                    _ => continue,
                };
                if !text.is_empty() {
                    initial_msgs.push(ChatMessage {
                        role: role.to_string(),
                        text: text.to_string(),
                    });
                }
            }
        }

        Self {
            messages: initial_msgs,
            input: String::new(),
            input_mode: InputMode::Insert,
            should_exit: false,
            status: "ready".to_string(),
            command_tx,
            event_rx,
            scroll_offset: 0,
            follow_bottom: true,
            picker_active: false,
            picker_prefix: String::new(),
            picker_query: String::new(),
            picker_results: Vec::new(),
            picker_selection: 0,
            picker_files: Vec::new(),
            picker_files_loaded: false,
            pending_permission: None,
            permission_rx,
            trust_store,
            approved: approve,
            token_used: 0,
            token_limit: crate::ai::token_count::get_context_limit(&model),
            near_limit: false,
            session_id,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(&mut stdout))?;
        crossterm::execute!(io::stdout(), EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;

        loop {
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(10))? {
                self.handle_event(event::read()?)?;
            }

            while let Ok((_idx, event)) = self.event_rx.try_recv() {
                self.handle_agent_event(event);
            }

            if let Ok(pending) = self.permission_rx.try_recv() {
                self.pending_permission = Some(pending);
            }

            if self.should_exit {
                break;
            }
        }

        let _ = self.command_tx.send(AppCommand::Exit);
        terminal::disable_raw_mode()?;
        crossterm::execute!(io::stdout(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture)?;
        Ok(())
    }

    fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TextDelta { text } => {
                if self.messages.is_empty()
                    || self.messages.last().map(|m| m.role.as_str()) != Some("assistant")
                {
                    self.messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        text: String::new(),
                    });
                }
                if let Some(last) = self.messages.last_mut() {
                    last.text.push_str(&text);
                }
                self.follow_bottom = true;
            }
            AgentEvent::ThinkingDelta { thinking } => {
                if let Some(last) = self.messages.last_mut() {
                    if !last.text.contains("💭") {
                        last.text.push_str(&format!("\n💭 {}...", thinking));
                    }
                }
            }
            AgentEvent::ToolUseStart { id: _, name } => {
                if let Some(last) = self.messages.last_mut() {
                    last.text.push_str(&format!("\n🛠 Using {}...", name));
                }
                self.follow_bottom = true;
            }
            AgentEvent::ToolResult { id: _, name, result } => {
                let mut preview: String = result.content.chars().take(100).collect();
                if preview.starts_with("Exit code: ") {
                    if let Some(rest) = preview.splitn(2, '\n').nth(1) {
                        preview = rest.chars().take(100).collect();
                    }
                }
                if result.is_error {
                    if let Some(last) = self.messages.last_mut() {
                        last.text.push_str(&format!("\n⚠️ [{}] {}", name, preview));
                    }
                } else {
                    if let Some(last) = self.messages.last_mut() {
                        last.text.push_str(&format!("\n✅ [{}] {}", name, preview));
                    }
                }
                self.follow_bottom = true;
            }
            AgentEvent::Error { message } => {
                if let Some(last) = self.messages.last_mut() {
                    last.text.push_str(&format!("\n❌ Error: {}", message));
                }
                self.status = "error".to_string();
                self.input_mode = InputMode::Insert;
            }
            AgentEvent::TurnEnd { stop_reason: _ } => {
                self.status = "ready".to_string();
            }
            AgentEvent::Done => {
                self.status = "ready".to_string();
                self.input_mode = InputMode::Insert;
                self.input.clear();
                self.near_limit = false;
            }
            AgentEvent::ToolUseDelta { input: _ } => {}
            AgentEvent::ContextWarning { fraction: _, used, limit } => {
                self.token_used = used;
                self.token_limit = limit;
                self.near_limit = true;
            }
            AgentEvent::TokenUpdate { used, limit } => {
                self.token_used = used;
                self.token_limit = limit;
            }
            AgentEvent::Compacting => {
                self.status = "compacting...".to_string();
            }
            AgentEvent::Compacted { summary: _ } => {
                self.status = "compacted".to_string();
                self.near_limit = false;
            }
        }
    }

    fn handle_event(&mut self, event: Event) -> io::Result<()> {
        match event {
            Event::Mouse(mouse) => {
                self.follow_bottom = false;
                match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        self.scroll_offset = self.scroll_offset.saturating_add(3);
                    }
                    MouseEventKind::ScrollUp => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(3);
                    }
                    _ => {}
                }
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if self.pending_permission.is_some() {
                    self.handle_permission_key(key);
                } else {
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.should_exit = true;
                        }
                        _ => match self.input_mode {
                            InputMode::Waiting => {}
                            InputMode::Normal => self.handle_normal_key(key),
                            InputMode::Insert => self.handle_insert_key(key),
                        },
                    }
                }
            },
            _ => {}
        }
        Ok(())
    }

    fn handle_normal_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Char('i') => self.input_mode = InputMode::Insert,
            KeyCode::Char('q') => self.should_exit = true,
            KeyCode::Up => {
                self.follow_bottom = false;
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            KeyCode::Down => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            KeyCode::PageUp => {
                self.follow_bottom = false;
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
            }
            _ => {}
        }
    }

    fn handle_insert_key(&mut self, key: crossterm::event::KeyEvent) {
        if self.picker_active {
            match key.code {
                KeyCode::Up => {
                    self.picker_selection = self.picker_selection.saturating_sub(1);
                }
                KeyCode::Down => {
                    let max = self.picker_results.len().saturating_sub(1);
                    self.picker_selection = self.picker_selection.saturating_add(1).min(max);
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if let Some(path) = self.picker_results.get(self.picker_selection).cloned() {
                        self.input = format!("{}{} ", self.picker_prefix, path);
                    }
                    self.picker_active = false;
                }
                KeyCode::Esc => {
                    self.input = self.picker_prefix.clone();
                    self.picker_active = false;
                }
                KeyCode::Backspace => {
                    if !self.picker_query.is_empty() {
                        self.picker_query.pop();
                        self.update_picker_results();
                    } else {
                        self.input = self.picker_prefix.clone();
                        self.picker_active = false;
                    }
                }
                KeyCode::Char(c) => {
                    self.picker_query.push(c);
                    self.update_picker_results();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Enter => {
                if !self.input.trim().is_empty() {
                    let text = std::mem::take(&mut self.input);
                    self.input.push_str(&text);
                    self.input_mode = InputMode::Waiting;

                    self.messages.push(ChatMessage {
                        role: "user".to_string(),
                        text: text.clone(),
                    });
                    self.messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        text: String::new(),
                    });

                    self.follow_bottom = true;
                    self.status = "thinking...".to_string();
                    let _ = self.command_tx.send(AppCommand::Submit { text });
                }
            }
            KeyCode::Char('@') => {
                self.start_picker();
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Esc => {
                self.picker_active = false;
                self.input_mode = InputMode::Normal;
            }
            _ => {}
        }
    }

    fn start_picker(&mut self) {
        self.picker_prefix = self.input.clone();
        self.picker_query = String::new();
        self.picker_selection = 0;
        self.picker_active = true;
        self.update_picker_results();
    }

    fn update_picker_results(&mut self) {
        if !self.picker_files_loaded {
            self.load_picker_files();
        }
        let query = self.picker_query.to_lowercase();
        self.picker_results = self
            .picker_files
            .iter()
            .filter(|f| query.is_empty() || f.to_lowercase().contains(&query))
            .take(20)
            .cloned()
            .collect();
        self.picker_selection = self
            .picker_selection
            .min(self.picker_results.len().saturating_sub(1));
    }

    fn load_picker_files(&mut self) {
        self.picker_files.clear();
        let cwd = std::env::current_dir().unwrap_or_default();
        for entry in WalkDir::new(&cwd)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
        {
            if let Ok(entry) = entry {
                if entry.file_type().is_file() {
                    if let Ok(relative) = entry.path().strip_prefix(&cwd) {
                        let path = relative.to_string_lossy().to_string();
                        if !path.is_empty() {
                            self.picker_files.push(path);
                        }
                    }
                }
            }
        }
        self.picker_files.sort();
        self.picker_files_loaded = true;
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(area);

        self.render_messages(frame, chunks[0]);
        self.render_input(frame, chunks[1]);
        self.render_status(frame, chunks[2]);
        if self.picker_active {
            self.render_picker(frame, area);
        }
        if self.pending_permission.is_some() {
            self.render_permission_prompt(frame, area);
        }
    }

    fn render_messages(&mut self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();
        for msg in &self.messages {
            let (prefix, color) = match msg.role.as_str() {
                "system" => ("◆ ", Color::Cyan),
                "user" => ("▶ ", Color::Green),
                "assistant" => ("▸ ", Color::Yellow),
                _ => ("  ", Color::White),
            };
            let bold_prefix = Style::default().fg(color).add_modifier(Modifier::BOLD);

            let rendered = render_markdown(&msg.text);
            for (i, line) in rendered.into_iter().enumerate() {
                let mut spans = line.spans;
                if i == 0 {
                    spans.insert(0, Span::styled(prefix, bold_prefix));
                }
                lines.push(Line::from(spans));
            }
            lines.push(Line::from(""));
        }

        let inner_height = (area.height as usize).saturating_sub(1);
        let total = lines.len();
        if self.follow_bottom || self.scroll_offset + inner_height > total {
            self.scroll_offset = total.saturating_sub(inner_height);
        }
        let start = self.scroll_offset.min(total.saturating_sub(inner_height));
        let visible_lines: Vec<Line> = lines.into_iter().skip(start).collect();

        let chat = Paragraph::new(visible_lines)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .title(" Chat ")
                    .title_alignment(ratatui::layout::Alignment::Center),
            );

        frame.render_widget(chat, area);
    }

    fn render_input(&mut self, frame: &mut Frame, area: Rect) {
        let mode_indicator = match self.input_mode {
            InputMode::Normal => " NORMAL ",
            InputMode::Insert => " INSERT ",
            InputMode::Waiting => " WAITING ",
        };

        let border_style = match self.input_mode {
            InputMode::Insert => Style::default().fg(Color::Green),
            InputMode::Waiting => Style::default().fg(Color::Yellow),
            _ => Style::default().fg(Color::DarkGray),
        };

        let display_text = if self.picker_active {
            format!("{}@{}", self.picker_prefix, self.picker_query)
        } else {
            self.input.clone()
        };

        let input = Paragraph::new(display_text.as_str())
            .style(match self.input_mode {
                InputMode::Waiting => Style::default().fg(Color::DarkGray),
                _ => Style::default().fg(Color::White),
            })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(mode_indicator)
                    .border_style(border_style),
            )
            .wrap(Wrap { trim: false });

        frame.render_widget(input, area);

        if self.input_mode == InputMode::Insert {
            let cursor_len = if self.picker_active {
                self.picker_prefix.len() + 1 + self.picker_query.len()
            } else {
                self.input.len()
            };
            let x = (cursor_len as u16 + 1).min(area.width.max(1).saturating_sub(2));
            frame.set_cursor_position(ratatui::layout::Position::new(area.x + x, area.y + 1));
        }
    }

    fn render_status(&mut self, frame: &mut Frame, area: Rect) {
        let status_color = if self.near_limit {
            Color::Red
        } else if self.status == "ready" {
            Color::Green
        } else {
            Color::Yellow
        };

        let token_str = if self.token_limit > 0 {
            let pct = self.token_used as f64 / self.token_limit as f64 * 100.0;
            if self.near_limit {
                format!(" ⚠ {:.0}%", pct)
            } else {
                format!(" {:.1}K/{}K", self.token_used as f64 / 1000.0, self.token_limit / 1000)
            }
        } else {
            String::new()
        };

        let sess = format!(" [{}]", &self.session_id);

        let status = Line::from(vec![
            Span::styled(" ^C quit", Style::default().fg(Color::DarkGray)),
            Span::styled(sess, Style::default().fg(Color::DarkGray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled(&self.status, Style::default().fg(status_color)),
            Span::styled(token_str, Style::default().fg(if self.near_limit { Color::Red } else { Color::DarkGray })),
        ]);
        frame.render_widget(Paragraph::new(status), area);
    }

    fn handle_permission_key(&mut self, key: crossterm::event::KeyEvent) {
        if let Some(pending) = self.pending_permission.take() {
            match key.code {
                KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Enter => {
                    let tool = pending.request.tool_name.clone();
                    let _ = pending.reply_tx.send(PermissionReply::Allow);
                    self.status = format!("allowed {}", tool);
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    let tool = pending.request.tool_name.clone();
                    let cwd = std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.trust_store.set_trusted(&cwd, true);
                    let _ = pending.reply_tx.send(PermissionReply::Allow);
                    self.status = format!("trusted project, allowed {}", tool);
                }
                KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Esc => {
                    let _ = pending.reply_tx.send(PermissionReply::Deny);
                    self.status = "denied".to_string();
                }
                _ => {
                    self.pending_permission = Some(pending);
                }
            }
        }
    }

    fn render_picker(&mut self, frame: &mut Frame, area: Rect) {
        if self.picker_results.is_empty() {
            return;
        }

        let picker_height = (self.picker_results.len() as u16).min(10).max(1);
        let picker_y = area.height.saturating_sub(4 + picker_height + 1);
        let picker_area = Rect {
            x: area.x + 1,
            y: area.y + picker_y,
            width: area.width.saturating_sub(2).min(60),
            height: picker_height + 2,
        };

        let items: Vec<ListItem> = self
            .picker_results
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let style = if i == self.picker_selection {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Cyan)
                };
                ListItem::new(path.as_str()).style(style)
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Files ")
                .border_style(Style::default().fg(Color::Cyan)),
        );

        frame.render_widget(list, picker_area);
    }

    fn render_permission_prompt(&mut self, frame: &mut Frame, area: Rect) {
        let pending = match self.pending_permission.as_ref() {
            Some(p) => p,
            None => return,
        };

        let tool_name = &pending.request.tool_name;
        let input_preview: String = pending
            .request
            .tool_input
            .chars()
            .take(120)
            .collect();

        let prompt_height = 8u16;
        let prompt_y = area.height.saturating_sub(4 + prompt_height + 2);
        let prompt_area = Rect {
            x: area.x + 2,
            y: area.y + prompt_y,
            width: area.width.saturating_sub(4).min(80),
            height: prompt_height,
        };

        let text = vec![
            Line::from(Span::styled(
                format!(" ⚠  {} requires approval", tool_name),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!(" {}", input_preview),
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(Span::styled(
                " (A)llow once  |  (T)rust project  |  (D)eny  ",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Permission ")
                    .border_style(Style::default().fg(Color::Yellow)),
            );

        frame.render_widget(paragraph, prompt_area);
    }
}
