mod json_highlight;

use std::io;
use std::num::NonZeroU32;
use std::sync::mpsc;

use chrono_humanize::HumanTime;
use conduit_core::{Config, Direction, Storages, TransitPage, TransitQuery, TransitRecord};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    TableState,
};

enum View {
    TransitListing,
    TransitDetail(usize),
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
    view: View,
    transit_records: Vec<TransitRecord>,
    latest_transit_stored_at: Option<chrono::DateTime<chrono::Utc>>,
    transit_table_state: TableState,
    auto_follow: bool,
    use_relative_time: bool,
    transit_detail_scroll: u16,
    is_loading: bool,
    poll_rx: mpsc::Receiver<PollMessage>,
    poll_abort: tokio::task::AbortHandle,
    error: Option<String>,
}

impl App {
    fn new(_config: Config, storages: Storages) -> Self {
        let (tx, rx) = mpsc::channel();

        let handle = tokio::runtime::Handle::current();
        let task = handle.spawn(poll_loop(storages, tx));

        Self {
            _config,
            should_quit: false,
            view: View::TransitListing,
            transit_records: Vec::new(),
            latest_transit_stored_at: None,
            transit_table_state: TableState::default(),
            auto_follow: true,
            use_relative_time: true,
            transit_detail_scroll: 0,
            is_loading: true,
            poll_rx: rx,
            poll_abort: task.abort_handle(),
            error: None,
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

    fn render(&mut self, frame: &mut ratatui::Frame) {
        match self.view {
            View::TransitListing => self.render_transit_listing(frame),
            View::TransitDetail(index) => self.render_transit_detail(frame, index),
        }
    }

    fn render_transit_listing(&mut self, frame: &mut ratatui::Frame) {
        let [table_area, status_area] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(frame.area());

        let header = Row::new(vec![
            Cell::from("Time"),
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
            .transit_records
            .iter()
            .map(|record| {
                let time_ago = if self.use_relative_time {
                    HumanTime::from(record.stored_at).to_string()
                } else {
                    record.stored_at.format("%Y-%m-%d %H:%M:%S").to_string()
                };

                let model_str = match record.model.as_deref() {
                    Some(m) => m.to_string(),
                    None => "-".to_string(),
                };

                let input_tokens = record
                    .usage
                    .as_ref()
                    .and_then(|u| u.get("prompt_tokens").or_else(|| u.get("input_tokens")))
                    .and_then(|v| v.as_u64())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string());

                let output_tokens = record
                    .usage
                    .as_ref()
                    .and_then(|u| {
                        u.get("completion_tokens")
                            .or_else(|| u.get("output_tokens"))
                    })
                    .and_then(|v| v.as_u64())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".to_string());

                Row::new(vec![
                    Cell::from(time_ago),
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
            Constraint::Length(19),
            Constraint::Length(20),
            Constraint::Length(12),
            Constraint::Length(13),
        ];
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().title(" Requests ").borders(Borders::ALL))
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_stateful_widget(table, table_area, &mut self.transit_table_state);

        let content_len = self.transit_records.len();
        let viewport = self.visible_table_rows(table_area.height);
        let scroll_pos = self.transit_table_state.selected().unwrap_or(0);
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
                "auto-follow: {} | q: quit, F: follow, T: {}, Enter/Right/l: details ",
                follow,
                if self.use_relative_time {
                    "absolute time"
                } else {
                    "relative time"
                },
            )
        };
        let status_style = if self.error.is_some() {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        frame.render_widget(Paragraph::new(status).style(status_style), status_area);
    }

    fn visible_table_rows(&self, area_height: u16) -> usize {
        area_height.saturating_sub(3) as usize // borders + header
    }

    fn render_transit_detail(&mut self, frame: &mut ratatui::Frame, index: usize) {
        let [content_area, status_area] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(frame.area());

        let record = &self.transit_records[index];

        let label_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);

        let mut lines = vec![
            Line::from(vec![
                Span::styled("Transit ID: ", label_style),
                Span::raw(record.transit_id.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Time:       ", label_style),
                Span::raw(
                    record
                        .stored_at
                        .format("%Y-%m-%d %H:%M:%S%.6f UTC")
                        .to_string(),
                ),
            ]),
            Line::from(vec![
                Span::styled("Header ID:  ", label_style),
                Span::raw(record.header_id.as_deref().unwrap_or("-").to_string()),
            ]),
            Line::from(vec![
                Span::styled("Body ID:    ", label_style),
                Span::raw(record.body_id.as_deref().unwrap_or("-").to_string()),
            ]),
            Line::from(vec![
                Span::styled("Model:      ", label_style),
                Span::raw(record.model.as_deref().unwrap_or("-").to_string()),
            ]),
        ];

        if let Some(ref usage) = record.usage {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled("Full Usage:", label_style)));
            lines.extend(json_highlight::json_to_lines(usage));
        }

        let content_len = lines.len();
        let viewport = content_area.height.saturating_sub(2) as usize;
        let max_scroll = content_len.saturating_sub(viewport) as u16;
        self.transit_detail_scroll = self.transit_detail_scroll.min(max_scroll);
        let detail = Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Transit Details ")
                    .borders(Borders::ALL),
            )
            .scroll((self.transit_detail_scroll, 0));
        frame.render_widget(detail, content_area);

        let mut scrollbar_state = ScrollbarState::new(content_len.saturating_sub(viewport))
            .position(self.transit_detail_scroll as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(scrollbar, content_area, &mut scrollbar_state);

        let position = format!(" {}/{} ", index + 1, self.transit_records.len());
        let status = format!(
            "{}| Backspace/Left/h: back, Up/Down: scroll, [/]: prev/next record ",
            position,
        );
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(Color::DarkGray)),
            status_area,
        );
    }

    fn handle_events(&mut self) -> anyhow::Result<()> {
        if event::poll(std::time::Duration::from_millis(100))? {
            match self.view {
                View::TransitListing => self.handle_listing_events()?,
                View::TransitDetail(_) => self.handle_detail_events()?,
            }
        }
        Ok(())
    }

    fn handle_listing_events(&mut self) -> anyhow::Result<()> {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                }
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    self.open_selected_detail();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.auto_follow = false;
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    self.transit_table_state.select(Some(i.saturating_sub(1)));
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let next = (i + 1).min(self.transit_records.len().saturating_sub(1));
                    self.transit_table_state.select(Some(next));
                    if next == self.transit_records.len().saturating_sub(1) {
                        self.auto_follow = true;
                    }
                }
                KeyCode::Home => {
                    self.auto_follow = false;
                    self.transit_table_state.select(Some(0));
                }
                KeyCode::End => {
                    self.auto_follow = true;
                    self.select_last();
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    self.use_relative_time = !self.use_relative_time;
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
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    self.transit_table_state.select(Some(i.saturating_sub(1)));
                }
                event::MouseEventKind::ScrollDown => {
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let next = (i + 1).min(self.transit_records.len().saturating_sub(1));
                    self.transit_table_state.select(Some(next));
                    if next == self.transit_records.len().saturating_sub(1) {
                        self.auto_follow = true;
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn open_selected_detail(&mut self) {
        if let Some(index) = self.transit_table_state.selected()
            && index < self.transit_records.len()
        {
            self.transit_detail_scroll = 0;
            self.view = View::TransitDetail(index);
        }
    }

    fn handle_detail_events(&mut self) -> anyhow::Result<()> {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                }
                KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                    self.view = View::TransitListing;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.transit_detail_scroll = self.transit_detail_scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.transit_detail_scroll = self.transit_detail_scroll.saturating_add(1);
                }
                KeyCode::PageUp => {
                    self.transit_detail_scroll =
                        self.transit_detail_scroll.saturating_sub(PAGE_SIZE as u16);
                }
                KeyCode::PageDown => {
                    self.transit_detail_scroll =
                        self.transit_detail_scroll.saturating_add(PAGE_SIZE as u16);
                }
                KeyCode::Home => {
                    self.transit_detail_scroll = 0;
                }
                KeyCode::End => {
                    self.transit_detail_scroll = u16::MAX;
                }
                KeyCode::Char('[') => {
                    if let View::TransitDetail(index) = self.view
                        && index > 0
                    {
                        let new_index = index - 1;
                        self.transit_detail_scroll = 0;
                        self.view = View::TransitDetail(new_index);
                        self.transit_table_state.select(Some(new_index));
                    }
                }
                KeyCode::Char(']') => {
                    if let View::TransitDetail(index) = self.view {
                        let max = self.transit_records.len().saturating_sub(1);
                        if index < max {
                            let new_index = index + 1;
                            self.transit_detail_scroll = 0;
                            self.view = View::TransitDetail(new_index);
                            self.transit_table_state.select(Some(new_index));
                        }
                    }
                }
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                event::MouseEventKind::ScrollUp => {
                    self.transit_detail_scroll = self.transit_detail_scroll.saturating_sub(3);
                }
                event::MouseEventKind::ScrollDown => {
                    self.transit_detail_scroll = self.transit_detail_scroll.saturating_add(3);
                }
                _ => {}
            },
            _ => {}
        }
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
                                .transit_records
                                .iter()
                                .any(|existing| existing.transit_id == r.transit_id)
                        })
                        .collect();
                    if !new_records.is_empty() {
                        self.latest_transit_stored_at = new_records.last().map(|r| r.stored_at);
                        self.transit_records.extend(new_records);
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
        if self.transit_records.is_empty() {
            self.transit_table_state.select(None);
        } else {
            self.transit_table_state
                .select(Some(self.transit_records.len() - 1));
        }
    }
}

const PAGE_SIZE: u32 = 50;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

enum PollMessage {
    Loading,
    Page(TransitPage),
    Error(String),
}

async fn poll_loop(storages: Storages, tx: mpsc::Sender<PollMessage>) {
    let limit = NonZeroU32::new(PAGE_SIZE).unwrap();

    let _ = tx.send(PollMessage::Loading);
    let initial = storages
        .transit
        .list_transits(TransitQuery {
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
            .transit
            .list_transits(TransitQuery {
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
