mod json_highlight;
pub mod logging;

use std::collections::HashSet;
use std::io;
use std::num::NonZeroU32;
use std::sync::mpsc;

use chrono_humanize::HumanTime;
use conduit_core::{Config, Direction, Storages, TransitPage, TransitQuery, TransitRecord};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use logging::LogBuffer;
use ratatui::Terminal;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    TableState,
};
use uuid::Uuid;

enum View {
    TransitListing,
    TransitDetail(usize),
}

pub fn start(config: Config, storages: Storages, log_buffer: LogBuffer) -> anyhow::Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |phi| {
        restore_terminal();
        prev_hook(phi);
    }));

    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    io::stdout().execute(crossterm::event::EnableMouseCapture)?;
    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;

    let mut app = App::new(config, storages, log_buffer);
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
    log_buffer: LogBuffer,
    log_area: ratatui::layout::Rect,
    mouse_position: (u16, u16),
    is_loading: bool,
    poll_rx: mpsc::Receiver<PollMessage>,
    poll_abort: tokio::task::AbortHandle,
}

impl App {
    fn new(_config: Config, storages: Storages, log_buffer: LogBuffer) -> Self {
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
            log_buffer,
            log_area: ratatui::layout::Rect::ZERO,
            mouse_position: (0, 0),
            is_loading: true,
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

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let log_lines: Vec<String> = self
            .log_buffer
            .lock()
            .map(|buf| buf.iter().cloned().collect())
            .unwrap_or_default();

        let log_height = if log_lines.is_empty() {
            0
        } else {
            log_lines.len() as u16 + 2 // +2 for the borders
        };

        let [content_area, log_area] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(log_height)])
                .areas(frame.area());

        match self.view {
            View::TransitListing => self.render_transit_listing(frame, content_area),
            View::TransitDetail(index) => self.render_transit_detail(frame, content_area, index),
        }

        self.log_area = log_area;
        self.render_log_panel(frame, log_area, log_lines);
    }

    fn render_log_panel(
        &self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        log_lines: Vec<String>,
    ) {
        if log_lines.is_empty() {
            return;
        }

        let log_text: Vec<Line> = log_lines
            .into_iter()
            .map(|line| {
                let style = if line.contains(" ERROR ") {
                    Style::default().fg(Color::Red)
                } else if line.contains(" WARN ") {
                    Style::default().fg(Color::Yellow)
                } else if line.contains(" INFO ") {
                    Style::default().fg(Color::LightBlue)
                } else if line.contains(" DEBUG ") {
                    Style::default().fg(Color::Gray)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                Line::styled(line, style)
            })
            .collect();

        let visible = area.height.saturating_sub(2) as usize;
        let skip = log_text.len().saturating_sub(visible);

        let clear_button_style = if self.is_log_close_hover() {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let log_widget = Paragraph::new(log_text.into_iter().skip(skip).collect::<Vec<_>>()).block(
            Block::default()
                .title(Line::from(vec![
                    Span::raw(" Logs "),
                    Span::styled("[ x to clear ]", clear_button_style),
                    Span::raw(" "),
                ]))
                .borders(Borders::ALL),
        );

        frame.render_widget(log_widget, area);
    }

    fn render_transit_listing(&mut self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        let [table_area, status_area] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(area);

        let header = Row::new(vec![
            Cell::from("Time"),
            Cell::from("Provider"),
            Cell::from("Model"),
            Cell::from("Cost"),
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

                let cost = record
                    .estimate_cost()
                    .map(|c| format!("${:.5}", c))
                    .unwrap_or_else(|| "-".to_string());

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
                    Cell::from(record.provider.to_string()),
                    Cell::from(model_str),
                    Cell::from(cost),
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
            Constraint::Length(12),
            Constraint::Length(20),
            Constraint::Length(12), // NB: leave room for hundreds and symbols before the decimals
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

        let status = if self.is_loading {
            " Loading... ".to_string()
        } else {
            format!(
                "Enter/Right/l: details, q: quit, f: go to latest, t: {}",
                if self.use_relative_time {
                    "absolute times"
                } else {
                    "relative times"
                },
            )
        };

        let status_style = Style::default().fg(Color::DarkGray);
        frame.render_widget(Paragraph::new(status).style(status_style), status_area);
    }

    fn visible_table_rows(&self, area_height: u16) -> usize {
        area_height.saturating_sub(3) as usize // borders + header
    }

    fn render_transit_detail(
        &mut self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        index: usize,
    ) {
        let [content_area, status_area] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(area);

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
                Span::styled("Provider:   ", label_style),
                Span::raw(record.provider.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Model:      ", label_style),
                Span::raw(record.model.as_deref().unwrap_or("-").to_string()),
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
                Span::styled("Est. Cost:  ", label_style),
                Span::raw(
                    record
                        .estimate_cost()
                        .map(|c| format!("${:.6}", c))
                        .unwrap_or_else(|| "-".to_string()),
                ),
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
                    .title(" Request Details ")
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
            "{}| Backspace/Left/h: back, Up/Down: scroll, [: previvous, ]: next",
            position,
        );
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(Color::DarkGray)),
            status_area,
        );
    }

    fn handle_events(&mut self) -> anyhow::Result<()> {
        if event::poll(std::time::Duration::from_millis(100))? {
            let ev = event::read()?;
            if let Event::Mouse(mouse) = &ev {
                self.mouse_position = (mouse.column, mouse.row);
            }
            match self.view {
                View::TransitListing => self.handle_listing_events(ev)?,
                View::TransitDetail(_) => self.handle_detail_events(ev)?,
            }
        }
        Ok(())
    }

    fn handle_listing_events(&mut self, ev: Event) -> anyhow::Result<()> {
        match ev {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                }
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    self.open_selected_detail();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let prev = i.saturating_sub(1);
                    self.transit_table_state.select(Some(prev));
                    if prev == 0 {
                        self.auto_follow = true;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.auto_follow = false;
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let next = (i + 1).min(self.transit_records.len().saturating_sub(1));
                    self.transit_table_state.select(Some(next));
                }
                KeyCode::PageUp => {
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let prev = i.saturating_sub(PG_BUTTON_JUMP);
                    self.transit_table_state.select(Some(prev));
                    if prev == 0 {
                        self.auto_follow = true;
                    }
                }
                KeyCode::PageDown => {
                    self.auto_follow = false;
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let next =
                        (i + PG_BUTTON_JUMP).min(self.transit_records.len().saturating_sub(1));
                    self.transit_table_state.select(Some(next));
                }
                KeyCode::Home => {
                    self.auto_follow = true;
                    self.select_first();
                }
                KeyCode::End => {
                    self.auto_follow = false;
                    if !self.transit_records.is_empty() {
                        self.transit_table_state
                            .select(Some(self.transit_records.len() - 1));
                    }
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    self.use_relative_time = !self.use_relative_time;
                }
                KeyCode::Char('f') | KeyCode::Char('F') => {
                    self.auto_follow = !self.auto_follow;
                    if self.auto_follow {
                        self.select_first();
                    }
                }
                KeyCode::Char('x') => {
                    self.clear_logs();
                }
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                event::MouseEventKind::ScrollUp => {
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let prev = i.saturating_sub(1);
                    self.transit_table_state.select(Some(prev));
                    if prev == 0 {
                        self.auto_follow = true;
                    }
                }
                event::MouseEventKind::ScrollDown => {
                    self.auto_follow = false;
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let next = (i + 1).min(self.transit_records.len().saturating_sub(1));
                    self.transit_table_state.select(Some(next));
                }
                event::MouseEventKind::Down(event::MouseButton::Left) => {
                    if self.is_log_close_hit(mouse.column, mouse.row) {
                        self.clear_logs();
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn clear_logs(&mut self) {
        if let Ok(mut buf) = self.log_buffer.lock() {
            buf.clear();
        }
    }

    fn is_log_close_hit(&self, column: u16, row: u16) -> bool {
        let area = self.log_area;
        if area.height == 0 {
            return false;
        }
        // " Logs [ x to clear ] "
        row == area.y && column >= area.x + 7 && column < area.x + 21
    }

    fn is_log_close_hover(&self) -> bool {
        let (col, row) = self.mouse_position;
        self.is_log_close_hit(col, row)
    }

    fn open_selected_detail(&mut self) {
        if let Some(index) = self.transit_table_state.selected()
            && index < self.transit_records.len()
        {
            self.transit_detail_scroll = 0;
            self.view = View::TransitDetail(index);
        }
    }

    fn handle_detail_events(&mut self, ev: Event) -> anyhow::Result<()> {
        match ev {
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
                    self.transit_detail_scroll = self
                        .transit_detail_scroll
                        .saturating_sub(PG_BUTTON_JUMP as u16);
                }
                KeyCode::PageDown => {
                    self.transit_detail_scroll = self
                        .transit_detail_scroll
                        .saturating_add(PG_BUTTON_JUMP as u16);
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
                event::MouseEventKind::Down(event::MouseButton::Left) => {
                    if self.is_log_close_hit(mouse.column, mouse.row) {
                        self.clear_logs();
                    }
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
                        self.latest_transit_stored_at = new_records.first().map(|r| r.stored_at);
                        let new_record_count = new_records.len();
                        self.transit_records.splice(0..0, new_records);
                        let in_detail = matches!(self.view, View::TransitDetail(_));
                        if self.auto_follow && !in_detail {
                            self.select_first();
                        } else {
                            if let Some(selected) = self.transit_table_state.selected() {
                                self.transit_table_state
                                    .select(Some(selected + new_record_count));
                            }
                            if let View::TransitDetail(ref mut index) = self.view {
                                *index += new_record_count;
                            }
                        }
                    }
                }
                PollMessage::Backfill(updated_records) => {
                    for updated in updated_records {
                        if let Some(existing) = self
                            .transit_records
                            .iter_mut()
                            .find(|r| r.transit_id == updated.transit_id)
                        {
                            existing.model = updated.model;
                            existing.usage = updated.usage;
                        }
                    }
                }
                PollMessage::Error(err) => {
                    self.is_loading = false;
                    tracing::error!("{}", err);
                }
            }
        }
    }

    fn select_first(&mut self) {
        if self.transit_records.is_empty() {
            self.transit_table_state.select(None);
        } else {
            self.transit_table_state.select(Some(0));
        }
    }
}

const PG_BUTTON_JUMP: usize = 10;
const PAGE_SIZE: u32 = 50;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

enum PollMessage {
    Loading,
    Page(TransitPage),
    Backfill(Vec<TransitRecord>),
    Error(String),
}

async fn poll_loop(storages: Storages, tx: mpsc::Sender<PollMessage>) {
    let limit = NonZeroU32::new(PAGE_SIZE).unwrap();
    let mut incomplete_ids: HashSet<Uuid> = HashSet::new();

    let _ = tx.send(PollMessage::Loading);
    let initial = storages
        .transit
        .list_transits(TransitQuery {
            cursor: None,
            direction: Direction::Before,
            limit,
        })
        .await;

    let mut cursor = match initial {
        Ok(page) => {
            let stored_at = page.records.first().map(|r| r.stored_at);
            track_incomplete(&mut incomplete_ids, &page.records);
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

        if !incomplete_ids.is_empty() {
            let ids: Vec<Uuid> = incomplete_ids.iter().copied().collect();
            if let Ok(records) = storages.transit.get_transits(ids).await {
                let filled: Vec<TransitRecord> =
                    records.into_iter().filter(|r| r.usage.is_some()).collect();
                for r in &filled {
                    incomplete_ids.remove(&r.transit_id);
                }
                if !filled.is_empty() {
                    let _ = tx.send(PollMessage::Backfill(filled));
                }
            }
        }

        let result = storages
            .transit
            .list_transits(TransitQuery {
                cursor,
                direction: Direction::After,
                limit,
            })
            .await;

        match result {
            Ok(page) => {
                if let Some(first) = page.records.first() {
                    cursor = Some(first.stored_at);
                }
                track_incomplete(&mut incomplete_ids, &page.records);
                let _ = tx.send(PollMessage::Page(page));
            }
            Err(e) => {
                let _ = tx.send(PollMessage::Error(e.to_string()));
            }
        }
    }
}

fn track_incomplete(incomplete_ids: &mut HashSet<Uuid>, records: &[TransitRecord]) {
    for record in records {
        // TODO: here we could also read from conduit config if usage is even meant to be recorded
        if record.usage.is_none() {
            incomplete_ids.insert(record.transit_id);
        } else {
            incomplete_ids.remove(&record.transit_id);
        }
    }
}
