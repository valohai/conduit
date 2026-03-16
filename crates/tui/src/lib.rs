use std::io;
use std::num::NonZeroU32;
use std::sync::mpsc;

use conduit_core::{Config, Direction, Storages, UsagePage, UsageQuery, UsageRecord};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{
    Block, Borders, Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState,
};

const PAGE_SIZE: u32 = 50;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

enum PollMessage {
    Loading,
    Page(UsagePage),
    Error(String),
}

pub fn start(config: Config, storages: Storages) -> anyhow::Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |phi| {
        restore_terminal();
        prev_hook(phi);
    }));

    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    io::stdout().execute(crossterm::event::EnableMouseCapture)?;
    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;

    let mut app = App::new(config, storages);
    let result = app.run(&mut terminal);
    restore_terminal();

    result
}

fn restore_terminal() {
    let _ = io::stdout().execute(crossterm::cursor::Show);
    let _ = io::stdout().execute(crossterm::event::DisableMouseCapture);
    let _ = io::stdout().execute(LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
}

struct App {
    _config: Config,
    should_quit: bool,
    usage_records: Vec<UsageRecord>,
    latest_usage_stored_at: Option<chrono::DateTime<chrono::Utc>>,
    table_state: TableState,
    auto_follow: bool,
    is_loading: bool,
    error: Option<String>,
    poll_rx: mpsc::Receiver<PollMessage>,
    poll_abort: tokio::task::AbortHandle,
}

impl App {
    fn new(_config: Config, storages: Storages) -> Self {
        let (tx, rx) = mpsc::channel();

        let handle = tokio::runtime::Handle::current();
        let task = handle.spawn(poll_loop(storages, tx));

        Self {
            _config,
            should_quit: false,
            usage_records: Vec::new(),
            latest_usage_stored_at: None,
            table_state: TableState::default(),
            auto_follow: true,
            is_loading: true,
            error: None,
            poll_rx: rx,
            poll_abort: task.abort_handle(),
        }
    }

    fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
        while !self.should_quit {
            self.process_poll_messages();
            terminal.draw(|frame| self.render(frame))?;
            self.handle_events()?;
        }
        self.poll_abort.abort();
        Ok(())
    }

    fn process_poll_messages(&mut self) {
        while let Ok(msg) = self.poll_rx.try_recv() {
            match msg {
                PollMessage::Loading => {
                    self.is_loading = true;
                }
                PollMessage::Page(page) => {
                    self.is_loading = false;
                    self.error = None;
                    let new_records: Vec<_> = page
                        .records
                        .into_iter()
                        .filter(|r| {
                            !self
                                .usage_records
                                .iter()
                                .any(|existing| existing.transit_id == r.transit_id)
                        })
                        .collect();
                    if !new_records.is_empty() {
                        self.latest_usage_stored_at = new_records.last().map(|r| r.stored_at);
                        self.usage_records.extend(new_records);
                        if self.auto_follow {
                            self.select_last();
                        }
                    }
                }
                PollMessage::Error(err) => {
                    self.is_loading = false;
                    self.error = Some(err);
                }
            }
        }
    }

    fn select_last(&mut self) {
        if self.usage_records.is_empty() {
            self.table_state.select(None);
        } else {
            self.table_state.select(Some(self.usage_records.len() - 1));
        }
    }

    fn visible_rows(&self, area_height: u16) -> usize {
        area_height.saturating_sub(3) as usize // borders + header
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let [table_area, status_area] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(frame.area());

        let header = Row::new(vec![
            Cell::from("TID"),
            Cell::from("Model"),
            Cell::from("Input Tokens"),
            Cell::from("Output Tokens"),
        ])
        .style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        );

        let mut rows: Vec<Row> = self
            .usage_records
            .iter()
            .map(|record| {
                let id = record.transit_id.to_string();
                let short_id = &id[id.len() - 8..];
                let model_str = match record.model.as_deref() {
                    Some(m) => m.to_string(),
                    None => "-".to_string(),
                };
                let input_tokens = record
                    .usage
                    .get("prompt_tokens")
                    .or_else(|| record.usage.get("input_tokens"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let output_tokens = record
                    .usage
                    .get("completion_tokens")
                    .or_else(|| record.usage.get("output_tokens"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string());
                Row::new(vec![
                    Cell::from(short_id.to_string()),
                    Cell::from(model_str),
                    Cell::from(input_tokens),
                    Cell::from(output_tokens),
                ])
            })
            .collect();

        if self.is_loading {
            rows.push(
                Row::new(vec![Cell::from(""), Cell::from("Loading...")]).style(
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ),
            );
        }

        let widths = [
            Constraint::Length(8),
            Constraint::Length(20),
            Constraint::Length(12),
            Constraint::Length(13),
        ];
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().title(" ⚡️ Conduit ").borders(Borders::ALL))
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_stateful_widget(table, table_area, &mut self.table_state);

        let content_len = self.usage_records.len();
        let viewport = self.visible_rows(table_area.height);
        let scroll_pos = self.table_state.selected().unwrap_or(0);
        let mut scrollbar_state = ScrollbarState::new(content_len.saturating_sub(viewport))
            .position(scroll_pos.saturating_sub(viewport / 2));
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(scrollbar, table_area, &mut scrollbar_state);

        let status = if let Some(ref err) = self.error {
            format!(" Error: {} ", err)
        } else if self.is_loading {
            " Loading... ".to_string()
        } else {
            let follow = if self.auto_follow { "ON" } else { "OFF" };
            format!(
                " {} records | auto-follow: {} | q: quit, F: toggle follow ",
                self.usage_records.len(),
                follow,
            )
        };
        let status_style = if self.error.is_some() {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        frame.render_widget(
            ratatui::widgets::Paragraph::new(status).style(status_style),
            status_area,
        );
    }

    fn handle_events(&mut self) -> anyhow::Result<()> {
        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                    KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        self.should_quit = true;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.auto_follow = false;
                        let i = self.table_state.selected().unwrap_or(0);
                        self.table_state.select(Some(i.saturating_sub(1)));
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let i = self.table_state.selected().unwrap_or(0);
                        let next = (i + 1).min(self.usage_records.len().saturating_sub(1));
                        self.table_state.select(Some(next));
                        if next == self.usage_records.len().saturating_sub(1) {
                            self.auto_follow = true;
                        }
                    }
                    KeyCode::Home => {
                        self.auto_follow = false;
                        self.table_state.select(Some(0));
                    }
                    KeyCode::End => {
                        self.auto_follow = true;
                        self.select_last();
                    }
                    KeyCode::Char('f') | KeyCode::Char('F') => {
                        self.auto_follow = !self.auto_follow;
                        if self.auto_follow {
                            self.select_last();
                        }
                    }
                    _ => {}
                },
                Event::Mouse(mouse) => match mouse.kind {
                    event::MouseEventKind::ScrollUp => {
                        self.auto_follow = false;
                        let i = self.table_state.selected().unwrap_or(0);
                        self.table_state.select(Some(i.saturating_sub(1)));
                    }
                    event::MouseEventKind::ScrollDown => {
                        let i = self.table_state.selected().unwrap_or(0);
                        let next = (i + 1).min(self.usage_records.len().saturating_sub(1));
                        self.table_state.select(Some(next));
                        if next == self.usage_records.len().saturating_sub(1) {
                            self.auto_follow = true;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(())
    }
}

async fn poll_loop(storages: Storages, tx: mpsc::Sender<PollMessage>) {
    let limit = NonZeroU32::new(PAGE_SIZE).unwrap();

    let _ = tx.send(PollMessage::Loading);
    let initial = storages
        .usage
        .list_usages(UsageQuery {
            cursor: None,
            direction: Direction::Older,
            limit,
        })
        .await;

    let mut cursor = match initial {
        Ok(page) => {
            let stored_at = page.records.last().map(|r| r.stored_at);
            let _ = tx.send(PollMessage::Page(page));
            stored_at
        }
        Err(e) => {
            let _ = tx.send(PollMessage::Error(e.to_string()));
            None
        }
    };

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        let _ = tx.send(PollMessage::Loading);
        let result = storages
            .usage
            .list_usages(UsageQuery {
                cursor,
                direction: Direction::Newer,
                limit,
            })
            .await;

        match result {
            Ok(page) => {
                if let Some(last) = page.records.last() {
                    cursor = Some(last.stored_at);
                }
                let _ = tx.send(PollMessage::Page(page));
            }
            Err(e) => {
                let _ = tx.send(PollMessage::Error(e.to_string()));
            }
        }
    }
}
