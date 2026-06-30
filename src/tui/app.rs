use crate::agent::r#loop::AgentEvent;
use crate::agent::state::AgentState;
use crate::agent::AgentLoop;
use crate::ai::provider::Provider;
use crossbeam_channel as channel;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use std::io;
use std::sync::Arc;
use std::time::Duration;

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

enum AppCommand {
    Submit { text: String },
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
}

impl App {
    pub fn new(provider: Arc<dyn Provider>, model: String, timeout_secs: u64) -> Self {
        let (command_tx, command_rx) = channel::unbounded::<AppCommand>();
        let (event_tx, event_rx) = channel::unbounded::<(usize, AgentEvent)>();

        let provider_name = provider.name().to_string();
        let provider2 = provider.clone();
        let model2 = model.clone();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let state = AgentState::new(model2, provider_name)
                    .with_system_prompt(
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
                            .to_string(),
                    );

                let mut agent_loop = AgentLoop::new(provider2, state);
                crate::tools::register_default_tools(&mut agent_loop);

                loop {
                    let cmd = command_rx.recv().unwrap_or(AppCommand::Exit);
                    match cmd {
                        AppCommand::Exit => break,
                        AppCommand::Submit { text } => {
                            let result = tokio::time::timeout(
                                timeout,
                                agent_loop.run(&text, &mut |event: AgentEvent| {
                                    let _ = event_tx.send((0, event));
                                }),
                            )
                            .await;
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
                        }
                    }
                }
            });
        });

        Self {
            messages: vec![ChatMessage {
                role: "system".to_string(),
                text: format!(
                    "Rs Agent - Minimalist AI Agent Toolkit\nModel: {}\n\nType a message to start a conversation.\ni: insert mode | Esc: normal mode | ^C: quit",
                    model
                ),
            }],
            input: String::new(),
            input_mode: InputMode::Insert,
            should_exit: false,
            status: "ready".to_string(),
            command_tx,
            event_rx,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(&mut stdout))?;
        crossterm::execute!(io::stdout(), EnterAlternateScreen)?;

        loop {
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(10))? {
                self.handle_event(event::read()?)?;
            }

            while let Ok((_idx, event)) = self.event_rx.try_recv() {
                self.handle_agent_event(event);
            }

            if self.should_exit {
                break;
            }
        }

        let _ = self.command_tx.send(AppCommand::Exit);
        terminal::disable_raw_mode()?;
        crossterm::execute!(io::stdout(), LeaveAlternateScreen)?;
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
            }
            AgentEvent::ToolUseDelta { input: _ } => {}
        }
    }

    fn handle_event(&mut self, event: Event) -> io::Result<()> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.should_exit = true;
                }
                _ => match self.input_mode {
                    InputMode::Waiting => {}
                    InputMode::Normal => self.handle_normal_key(key),
                    InputMode::Insert => self.handle_insert_key(key),
                },
            },
            _ => {}
        }
        Ok(())
    }

    fn handle_normal_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Char('i') => self.input_mode = InputMode::Insert,
            KeyCode::Char('q') => self.should_exit = true,
            _ => {}
        }
    }

    fn handle_insert_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                if !self.input.trim().is_empty() {
                    let text = std::mem::take(&mut self.input);
                    self.input_mode = InputMode::Waiting;

                    self.messages.push(ChatMessage {
                        role: "user".to_string(),
                        text: text.clone(),
                    });
                    self.messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        text: String::new(),
                    });

                    self.status = "thinking...".to_string();
                    let _ = self.command_tx.send(AppCommand::Submit { text });
                }
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Esc => self.input_mode = InputMode::Normal,
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame) {
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
    }

    fn render_messages(&self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();
        for msg in &self.messages {
            let (prefix, color) = match msg.role.as_str() {
                "system" => ("◆ ", Color::Cyan),
                "user" => ("▶ ", Color::Green),
                "assistant" => ("▸ ", Color::Yellow),
                _ => ("  ", Color::White),
            };
            let style = Style::default().fg(color);
            let bold_style = style.add_modifier(Modifier::BOLD);

            let text = Text::from(msg.text.as_str());
            for (i, line) in text.lines.into_iter().enumerate() {
                let mut spans: Vec<Span> = line
                    .spans
                    .into_iter()
                    .map(|s| s.style(style))
                    .collect();
                if i == 0 {
                    spans.insert(0, Span::styled(prefix, bold_style));
                }
                lines.push(Line::from(spans));
            }
        }

        let total_lines = lines.len();
        let visible_rows = (area.height as usize).saturating_sub(1);
        let scroll_offset = total_lines.saturating_sub(visible_rows) as u16;

        let chat = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .title(" Chat ")
                    .title_alignment(ratatui::layout::Alignment::Center),
            )
            .wrap(Wrap { trim: false })
            .scroll((0, scroll_offset));

        frame.render_widget(chat, area);
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
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

        let input = Paragraph::new(self.input.as_str())
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
            let x = (self.input.len() as u16 + 1).min(area.width.max(1) - 2);
            frame.set_cursor_position(ratatui::layout::Position::new(area.x + x, area.y + 1));
        }
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let status = Line::from(vec![
            Span::styled(" ^C quit | ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &self.status,
                Style::default().fg(if self.status == "ready" {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
        ]);
        frame.render_widget(Paragraph::new(status), area);
    }
}
