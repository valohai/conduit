mod json_highlight;
pub mod logging;
pub mod theme;

use std::collections::HashSet;
use std::io;
use std::num::NonZeroU32;
use std::sync::mpsc;

use chrono_humanize::HumanTime;
use conduit_core::{
    Config, Direction, Provider, Storages, TransitPage, TransitQuery, TransitRecord,
};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use logging::LogBuffer;
use ratatui::Terminal;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Table, TableState,
};
use theme::Theme;
use uuid::Uuid;

/// Format a u64 with thousand separators (e.g. 1234567 -> "1,234,567").
fn format_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

trait AsF64: Copy {
    fn as_f64(self) -> f64;
}
impl AsF64 for f64 {
    fn as_f64(self) -> f64 {
        self
    }
}
impl AsF64 for u64 {
    fn as_f64(self) -> f64 {
        self as f64
    }
}

#[inline]
fn update_range<T: AsF64>(range: &mut Option<(T, T)>, value: Option<T>) {
    if let Some(v) = value {
        *range = Some(match *range {
            Some((min, max)) => {
                let vf = v.as_f64();
                (
                    if vf < min.as_f64() { v } else { min },
                    if vf > max.as_f64() { v } else { max },
                )
            }
            None => (v, v),
        });
    }
}

/// Build a right-aligned numeric cell with an optional data-bar background.
/// When `range` is `Some((min, max))`, the cell background is filled
/// proportionally to where `value` falls in that range.
fn numeric_cell<'a, T: AsF64>(
    value: Option<T>,
    fmt: fn(T) -> String,
    width: usize,
    range: Option<(T, T)>,
    bar_style: Style,
) -> Cell<'a> {
    let text = value.map(fmt).unwrap_or_else(|| "-".into());
    if let (Some(v), Some((min, max))) = (value, range) {
        let span = max.as_f64() - min.as_f64();
        let ratio = if span > 0.0 {
            (v.as_f64() - min.as_f64()) / span
        } else {
            1.0
        };
        let padded = format!("{text:>width$}");
        let bar_chars = ((ratio * width as f64).round() as usize).min(width);
        let (bar_part, rest_part) = padded.split_at(bar_chars);
        Cell::from(Line::from(vec![
            Span::styled(bar_part.to_owned(), bar_style),
            Span::raw(rest_part.to_owned()),
        ]))
    } else {
        Cell::from(Line::from(text).alignment(Alignment::Right))
    }
}

/// Pre-extracted display fields from a `TransitRecord`.
struct DisplayTransitRecord<'a> {
    time: String,
    provider: &'a Provider,
    model: Option<&'a str>,
    cost: Option<f64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl<'a> DisplayTransitRecord<'a> {
    fn from_record(record: &'a TransitRecord, relative_time: bool) -> Self {
        let input_tokens = record
            .usage
            .as_ref()
            .and_then(|u| u.get("prompt_tokens").or_else(|| u.get("input_tokens")))
            .and_then(|v| v.as_u64());
        let output_tokens = record
            .usage
            .as_ref()
            .and_then(|u| {
                u.get("completion_tokens")
                    .or_else(|| u.get("output_tokens"))
            })
            .and_then(|v| v.as_u64());
        Self {
            time: if relative_time {
                HumanTime::from(record.stored_at).to_string()
            } else {
                record.stored_at.format("%Y-%m-%d %H:%M:%S").to_string()
            },
            provider: &record.provider,
            model: record.model.as_deref(),
            cost: record.estimate_cost(),
            input_tokens,
            output_tokens,
        }
    }
}

enum View {
    TransitListing,
    TransitDetail(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenMenu {
    File,
    Help,
}

pub fn start(
    config: Config,
    storages: Storages,
    log_buffer: LogBuffer,
    theme: Theme,
) -> anyhow::Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |phi| {
        restore_terminal();
        prev_hook(phi);
    }));

    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    io::stdout().execute(crossterm::event::EnableMouseCapture)?;
    let mut terminal = Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;

    let mut app = App::new(config, storages, log_buffer, theme);
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
    storages: Storages,
    theme: Theme,
    should_quit: bool,
    view: View,
    transit_records: Vec<TransitRecord>,
    latest_transit_stored_at: Option<chrono::DateTime<chrono::Utc>>,
    oldest_transit_stored_at: Option<chrono::DateTime<chrono::Utc>>,
    has_older_records: bool,
    is_loading_newer: bool,
    is_loading_older: bool,
    transit_table_state: TableState,
    auto_follow: bool,
    unseen_count: usize,
    use_relative_time: bool,
    show_data_bars: bool,
    transit_detail_scroll: u16,
    log_buffer: LogBuffer,
    log_area: Rect,
    mouse_position: (u16, u16),
    open_menu: Option<OpenMenu>,
    show_about: bool,
    menubar_area: Rect,
    tick: u64,
    poll_tx: mpsc::Sender<PollMessage>,
    poll_rx: mpsc::Receiver<PollMessage>,
    poll_abort: tokio::task::AbortHandle,
}

impl App {
    fn new(_config: Config, storages: Storages, log_buffer: LogBuffer, theme: Theme) -> Self {
        let (tx, rx) = mpsc::channel();

        let handle = tokio::runtime::Handle::current();
        let poll_tx = tx.clone();
        let poll_storages = storages.clone();
        let task = handle.spawn(poll_loop(poll_storages, tx));

        Self {
            _config,
            storages,
            theme,
            should_quit: false,
            view: View::TransitListing,
            transit_records: Vec::new(),
            latest_transit_stored_at: None,
            oldest_transit_stored_at: None,
            has_older_records: true,
            is_loading_older: false,
            transit_table_state: TableState::default(),
            auto_follow: true,
            unseen_count: 0,
            use_relative_time: true,
            show_data_bars: true,
            transit_detail_scroll: 0,
            log_buffer,
            log_area: Rect::ZERO,
            mouse_position: (0, 0),
            open_menu: None,
            show_about: false,
            menubar_area: Rect::ZERO,
            tick: 0,
            is_loading_newer: true,
            poll_tx,
            poll_rx: rx,
            poll_abort: task.abort_handle(),
        }
    }

    fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
        while !self.should_quit {
            self.process_poll_messages();
            self.tick = self.tick.wrapping_add(1);
            terminal.draw(|frame| self.render(frame))?;
            self.handle_events()?;
        }
        self.poll_abort.abort();
        Ok(())
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        // Paint the base style (background) across the whole frame
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(Block::default().style(self.theme.base), area);

        let has_menubar = self.theme.menubar.is_some();
        let menubar_height = if has_menubar { 1 } else { 0 };

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

        let chunks = Layout::vertical([
            Constraint::Length(menubar_height),
            Constraint::Min(5),
            Constraint::Length(log_height),
        ])
        .split(area);
        let menubar_area = chunks[0];
        let content_area = chunks[1];
        let log_area = chunks[2];

        self.menubar_area = menubar_area;
        if has_menubar {
            self.render_menubar(frame, menubar_area);
        }

        match self.view {
            View::TransitListing => self.render_transit_listing(frame, content_area),
            View::TransitDetail(index) => self.render_transit_detail(frame, content_area, index),
        }

        self.log_area = log_area;
        self.render_log_panel(frame, log_area, log_lines);

        // Render menu dropdowns and about dialog on top of everything
        if has_menubar {
            self.render_menu_dropdowns(frame, menubar_area);
        }
        if self.show_about {
            self.render_about_dialog(frame, area);
        }
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

        let theme = &self.theme;
        let log_text: Vec<Line> = log_lines
            .into_iter()
            .map(|line| {
                let style = if line.contains(" ERROR ") {
                    theme.log_error
                } else if line.contains(" WARN ") {
                    theme.log_warn
                } else if line.contains(" INFO ") {
                    theme.log_info
                } else if line.contains(" DEBUG ") {
                    theme.log_debug
                } else {
                    theme.log_other
                };
                Line::styled(line, style)
            })
            .collect();

        let visible = area.height.saturating_sub(2) as usize;
        let skip = log_text.len().saturating_sub(visible);

        let clear_button_style = if self.is_log_close_hover() {
            theme.log_close_hover
        } else {
            theme.log_close_normal
        };
        let log_widget = Paragraph::new(log_text.into_iter().skip(skip).collect::<Vec<_>>()).block(
            Block::default()
                .title(Line::from(vec![
                    Span::raw(" Logs "),
                    Span::styled("[ x to clear ]", clear_button_style),
                    Span::raw(" "),
                ]))
                .borders(Borders::ALL)
                .border_style(theme.border)
                .title_style(theme.title)
                .title_alignment(theme.title_alignment),
        );

        frame.render_widget(log_widget, area);
    }

    /// X position of the " Help" label on the menubar (right-aligned).
    fn help_menu_x(&self) -> u16 {
        // " Help " is 6 chars, pinned to right edge
        self.menubar_area.x + self.menubar_area.width.saturating_sub(HELP_LABEL_WIDTH)
    }

    fn render_menubar(&self, frame: &mut ratatui::Frame, area: Rect) {
        let style = self.theme.menubar.unwrap_or(self.theme.base);
        let hotkey = self.theme.menubar_hotkey;
        let sel = self.theme.menubar_selected;

        // Fill the bar
        let bar = Paragraph::new("").style(style);
        frame.render_widget(bar, area);

        let file_style = if self.open_menu == Some(OpenMenu::File) {
            sel
        } else {
            style
        };
        let file_hotkey = if self.open_menu == Some(OpenMenu::File) {
            sel
        } else {
            hotkey
        };

        // " File" on the left
        let file_spans = Line::from(vec![
            Span::styled(" ", file_style),
            Span::styled("F", file_hotkey),
            Span::styled("ile", file_style),
            Span::styled(" ", file_style),
        ]);
        let file_area = Rect::new(area.x, area.y, FILE_LABEL_WIDTH, 1);
        frame.render_widget(Paragraph::new(file_spans), file_area);

        // " Help " on the right
        let help_style = if self.open_menu == Some(OpenMenu::Help) {
            sel
        } else {
            style
        };
        let help_hotkey = if self.open_menu == Some(OpenMenu::Help) {
            sel
        } else {
            hotkey
        };

        let help_x = self.help_menu_x();
        let help_spans = Line::from(vec![
            Span::styled(" ", help_style),
            Span::styled("H", help_hotkey),
            Span::styled("elp", help_style),
            Span::styled(" ", help_style),
        ]);
        let help_area = Rect::new(help_x, area.y, HELP_LABEL_WIDTH, 1);
        frame.render_widget(Paragraph::new(help_spans), help_area);

        // Animated decoration in the center-right (between menus)
        let (decoration, dec_width) = self.theme.menubar_decoration.frame(self.tick);
        if dec_width > 0 {
            let dec_style = self.theme.menubar_decoration_style;
            let avail_start = area.x + FILE_LABEL_WIDTH;
            let avail_end = help_x;
            if avail_end > avail_start + dec_width {
                // Center it in the available space
                let mid = avail_start + (avail_end - avail_start - dec_width) / 2;
                let dec_area = Rect::new(mid, area.y, dec_width, 1);
                frame.render_widget(
                    Paragraph::new(Span::styled(decoration, dec_style)),
                    dec_area,
                );
            }
        }
    }

    fn render_menu_dropdowns(&self, frame: &mut ratatui::Frame, menubar_area: Rect) {
        let style = self.theme.menu_dropdown;
        let sel = self.theme.menu_dropdown_selected;

        match self.open_menu {
            Some(OpenMenu::File) => {
                // Dropdown below " File" (x=1)
                let drop = Rect::new(
                    menubar_area.x,
                    menubar_area.y + 1,
                    12, // " Quit  Alt+Q"
                    3,  // top border + item + bottom border
                );
                let is_hover = self.menu_dropdown_hover(drop) == Some(0);
                let item_style = if is_hover { sel } else { style };
                let block = Block::default().borders(Borders::ALL).style(style);
                frame.render_widget(Clear, drop);
                frame.render_widget(block, drop);
                let item_area = Rect::new(drop.x + 1, drop.y + 1, drop.width - 2, 1);
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(" Quit    ", item_style)])),
                    item_area,
                );
            }
            Some(OpenMenu::Help) => {
                // Dropdown below " Help" (right-aligned)
                let help_x = self.help_menu_x();
                let drop_x = help_x.saturating_sub(12 - HELP_LABEL_WIDTH);
                let drop = Rect::new(drop_x, menubar_area.y + 1, 12, 3);
                let is_hover = self.menu_dropdown_hover(drop) == Some(0);
                let item_style = if is_hover { sel } else { style };
                let block = Block::default().borders(Borders::ALL).style(style);
                frame.render_widget(Clear, drop);
                frame.render_widget(block, drop);
                let item_area = Rect::new(drop.x + 1, drop.y + 1, drop.width - 2, 1);
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(" About   ", item_style)])),
                    item_area,
                );
            }
            None => {}
        }
    }

    fn render_about_dialog(&self, frame: &mut ratatui::Frame, area: Rect) {
        let w = 40u16;
        let h = 7u16;
        let x = area.x + area.width.saturating_sub(w) / 2;
        let y = area.y + area.height.saturating_sub(h) / 2;
        let dialog = Rect::new(x, y, w.min(area.width), h.min(area.height));

        let style = self.theme.menu_dropdown;
        let title_style = self.theme.title;

        frame.render_widget(Clear, dialog);
        let text = vec![
            Line::raw(""),
            Line::from("Conduit").alignment(Alignment::Center),
            Line::raw(""),
            Line::from("API Transit Dashboard").alignment(Alignment::Center),
            Line::from("Press any key to close").alignment(Alignment::Center),
        ];
        let block = Block::default()
            .title(" About ")
            .title_alignment(Alignment::Center)
            .title_style(title_style)
            .borders(Borders::ALL)
            .style(style);
        frame.render_widget(Paragraph::new(text).block(block), dialog);
    }

    /// Returns which item index (0-based) the mouse is hovering over inside a dropdown.
    fn menu_dropdown_hover(&self, drop: Rect) -> Option<usize> {
        let (mx, my) = self.mouse_position;
        // Items start at drop.y + 1 (after top border), inside drop.x+1..drop.x+w-1
        if mx > drop.x
            && mx < drop.x + drop.width - 1
            && my > drop.y
            && my < drop.y + drop.height - 1
        {
            Some((my - drop.y - 1) as usize)
        } else {
            None
        }
    }

    fn render_transit_listing(&mut self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        let [table_area, status_area] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(area);

        let theme = &self.theme;

        let header = Row::new(vec![
            Cell::from("Time"),
            Cell::from("Provider"),
            Cell::from("Model"),
            Cell::from(Line::from("Cost").alignment(Alignment::Right)),
            Cell::from(Line::from("Input Tokens").alignment(Alignment::Right)),
            Cell::from(Line::from("Output Tokens").alignment(Alignment::Right)),
        ])
        .style(theme.table_header);

        let bar_style = theme.data_bar;

        let mut cost_range: Option<(f64, f64)> = None;
        let mut input_range: Option<(u64, u64)> = None;
        let mut output_range: Option<(u64, u64)> = None;
        let display_records: Vec<DisplayTransitRecord> = self
            .transit_records
            .iter()
            .map(|r| {
                let dr = DisplayTransitRecord::from_record(r, self.use_relative_time);
                if self.show_data_bars {
                    update_range(&mut cost_range, dr.cost);
                    update_range(&mut input_range, dr.input_tokens);
                    update_range(&mut output_range, dr.output_tokens);
                }
                dr
            })
            .collect();

        let mut rows: Vec<Row> = display_records
            .iter()
            .map(|dr| {
                Row::new(vec![
                    Cell::from(dr.time.as_str()),
                    Cell::from(dr.provider.to_string()),
                    Cell::from(dr.model.unwrap_or("-")),
                    numeric_cell(dr.cost, |c| format!("${:.5}", c), 12, cost_range, bar_style),
                    numeric_cell(
                        dr.input_tokens,
                        format_thousands,
                        12,
                        input_range,
                        bar_style,
                    ),
                    numeric_cell(
                        dr.output_tokens,
                        format_thousands,
                        13,
                        output_range,
                        bar_style,
                    ),
                ])
            })
            .collect();

        if self.is_loading_newer {
            rows.push(
                Row::new(vec![Cell::from(""), Cell::from("Loading...")]).style(theme.loading),
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
        let title = if self.unseen_count > 0 {
            Line::from(vec![
                Span::raw(" Requests "),
                Span::styled(
                    theme
                        .unseen_badge_fmt
                        .replace("{}", &self.unseen_count.to_string()),
                    theme.unseen_badge,
                ),
            ])
        } else {
            Line::from(" Requests ")
        };
        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(theme.border)
                    .title_style(theme.title)
                    .title_alignment(theme.title_alignment),
            )
            .row_highlight_style(theme.row_highlight);

        frame.render_stateful_widget(table, table_area, &mut self.transit_table_state);

        let content_len = self.transit_records.len();
        let viewport = self.visible_table_rows(table_area.height);
        let scroll_pos = self.transit_table_state.selected().unwrap_or(0);
        let mut scrollbar_state = ScrollbarState::new(content_len.saturating_sub(viewport))
            .position(scroll_pos.saturating_sub(viewport / 2));
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(scrollbar, table_area, &mut scrollbar_state);

        let status = format!(
            "Enter/Right/l: details, q: quit, f: go to latest, t: {}, b: data bars {}",
            if self.use_relative_time {
                "absolute times"
            } else {
                "relative times"
            },
            if self.show_data_bars { "off" } else { "on" },
        );

        frame.render_widget(Paragraph::new(status).style(theme.status), status_area);
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

        let theme = &self.theme;
        let record = &self.transit_records[index];

        let label_style = theme.label;

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

        if let Some(ref vh) = record.vh_headers {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled("Valohai Headers:", label_style)));
            let mut keys: Vec<_> = vh.keys().collect();
            keys.sort();
            for key in keys {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}: ", key), label_style),
                    Span::raw(vh[key].clone()),
                ]));
            }
        }

        if let Some(ref usage) = record.usage {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled("Full Usage:", label_style)));
            lines.extend(json_highlight::json_to_lines(usage, theme));
        }

        let content_len = lines.len();
        let viewport = content_area.height.saturating_sub(2) as usize;
        let max_scroll = content_len.saturating_sub(viewport) as u16;
        self.transit_detail_scroll = self.transit_detail_scroll.min(max_scroll);
        let detail = Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Request Details ")
                    .borders(Borders::ALL)
                    .border_style(theme.border)
                    .title_style(theme.title)
                    .title_alignment(theme.title_alignment),
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
        frame.render_widget(Paragraph::new(status).style(theme.status), status_area);
    }

    fn handle_events(&mut self) -> anyhow::Result<()> {
        if event::poll(std::time::Duration::from_millis(100))? {
            let ev = event::read()?;
            if let Event::Mouse(mouse) = &ev {
                self.mouse_position = (mouse.column, mouse.row);
            }

            // About dialog eats all input
            if self.show_about {
                if matches!(ev, Event::Key(key) if key.kind == KeyEventKind::Press)
                    || matches!(ev, Event::Mouse(m) if m.kind == event::MouseEventKind::Down(event::MouseButton::Left))
                {
                    self.show_about = false;
                }
                return Ok(());
            }

            // Menu bar events (when a menu is open or Alt shortcuts)
            if self.handle_menu_events(&ev)? {
                return Ok(());
            }

            match self.view {
                View::TransitListing => self.handle_listing_events(ev)?,
                View::TransitDetail(_) => self.handle_detail_events(ev)?,
            }
        }
        Ok(())
    }

    /// Handle menu-related events. Returns true if the event was consumed.
    fn handle_menu_events(&mut self, ev: &Event) -> anyhow::Result<bool> {
        let has_menubar = self.theme.menubar.is_some();

        // Alt+F / Alt+H to open menus
        if let Event::Key(key) = ev
            && key.kind == KeyEventKind::Press
            && has_menubar
        {
            if key.modifiers.contains(event::KeyModifiers::ALT) {
                match key.code {
                    KeyCode::Char('f') | KeyCode::Char('F') => {
                        self.open_menu = if self.open_menu == Some(OpenMenu::File) {
                            None
                        } else {
                            Some(OpenMenu::File)
                        };
                        return Ok(true);
                    }
                    KeyCode::Char('h') | KeyCode::Char('H') => {
                        self.open_menu = if self.open_menu == Some(OpenMenu::Help) {
                            None
                        } else {
                            Some(OpenMenu::Help)
                        };
                        return Ok(true);
                    }
                    _ => {}
                }
            }

            // When a menu is open, handle navigation
            if self.open_menu.is_some() {
                match key.code {
                    KeyCode::Esc => {
                        self.open_menu = None;
                        return Ok(true);
                    }
                    KeyCode::Left | KeyCode::Right => {
                        self.open_menu = Some(match self.open_menu {
                            Some(OpenMenu::File) => OpenMenu::Help,
                            _ => OpenMenu::File,
                        });
                        return Ok(true);
                    }
                    KeyCode::Enter => {
                        match self.open_menu {
                            Some(OpenMenu::File) => self.should_quit = true,
                            Some(OpenMenu::Help) => {
                                self.show_about = true;
                                self.open_menu = None;
                            }
                            None => {}
                        }
                        return Ok(true);
                    }
                    _ => {
                        self.open_menu = None;
                        return Ok(true);
                    }
                }
            }
        }

        // Mouse clicks on the menu bar and dropdowns
        if let Event::Mouse(mouse) = ev {
            if mouse.kind == event::MouseEventKind::Down(event::MouseButton::Left) && has_menubar {
                let (mx, my) = (mouse.column, mouse.row);

                // Click on menu bar (row 0)?
                if my == 0 {
                    let help_x = self.help_menu_x();
                    if mx < FILE_LABEL_WIDTH {
                        // " File"
                        self.open_menu = if self.open_menu == Some(OpenMenu::File) {
                            None
                        } else {
                            Some(OpenMenu::File)
                        };
                        return Ok(true);
                    } else if mx >= help_x && mx < help_x + HELP_LABEL_WIDTH {
                        // " Help"
                        self.open_menu = if self.open_menu == Some(OpenMenu::Help) {
                            None
                        } else {
                            Some(OpenMenu::Help)
                        };
                        return Ok(true);
                    } else if self.open_menu.is_some() {
                        self.open_menu = None;
                        return Ok(true);
                    }
                }

                // Click inside an open dropdown?
                if self.open_menu == Some(OpenMenu::File) {
                    let drop = Rect::new(0, 1, 12, 3);
                    if let Some(0) = self.menu_dropdown_hover_at(drop, mx, my) {
                        self.should_quit = true;
                        return Ok(true);
                    } else {
                        self.open_menu = None;
                        return Ok(true);
                    }
                }
                if self.open_menu == Some(OpenMenu::Help) {
                    let help_x = self.help_menu_x();
                    let drop_x = help_x.saturating_sub(12 - HELP_LABEL_WIDTH);
                    let drop = Rect::new(drop_x, 1, 12, 3);
                    if let Some(0) = self.menu_dropdown_hover_at(drop, mx, my) {
                        self.show_about = true;
                        self.open_menu = None;
                        return Ok(true);
                    } else {
                        self.open_menu = None;
                        return Ok(true);
                    }
                }
            }

            // Any click outside when menu is open closes it
            if mouse.kind == event::MouseEventKind::Down(event::MouseButton::Left)
                && self.open_menu.is_some()
            {
                self.open_menu = None;
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn menu_dropdown_hover_at(&self, drop: Rect, mx: u16, my: u16) -> Option<usize> {
        if mx > drop.x
            && mx < drop.x + drop.width - 1
            && my > drop.y
            && my < drop.y + drop.height - 1
        {
            Some((my - drop.y - 1) as usize)
        } else {
            None
        }
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
                        self.unseen_count = 0;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.auto_follow = false;
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let next = (i + 1).min(self.transit_records.len().saturating_sub(1));
                    self.transit_table_state.select(Some(next));
                    self.request_older_records_if_needed();
                }
                KeyCode::PageUp => {
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let prev = i.saturating_sub(PG_BUTTON_JUMP);
                    self.transit_table_state.select(Some(prev));
                    if prev == 0 {
                        self.auto_follow = true;
                        self.unseen_count = 0;
                    }
                }
                KeyCode::PageDown => {
                    self.auto_follow = false;
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let next =
                        (i + PG_BUTTON_JUMP).min(self.transit_records.len().saturating_sub(1));
                    self.transit_table_state.select(Some(next));
                    self.request_older_records_if_needed();
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
                    self.request_older_records_if_needed();
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    self.use_relative_time = !self.use_relative_time;
                }
                KeyCode::Char('b') | KeyCode::Char('B') => {
                    self.show_data_bars = !self.show_data_bars;
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
                        self.unseen_count = 0;
                    }
                }
                event::MouseEventKind::ScrollDown => {
                    self.auto_follow = false;
                    let i = self.transit_table_state.selected().unwrap_or(0);
                    let next = (i + 1).min(self.transit_records.len().saturating_sub(1));
                    self.transit_table_state.select(Some(next));
                    self.request_older_records_if_needed();
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
                    self.is_loading_newer = true;
                }
                PollMessage::Page(page) => {
                    self.is_loading_newer = false;
                    if self.oldest_transit_stored_at.is_none() {
                        self.has_older_records = page.has_more;
                    }
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
                        if self.oldest_transit_stored_at.is_none() {
                            self.oldest_transit_stored_at = new_records.last().map(|r| r.stored_at);
                        }
                        let new_record_count = new_records.len();
                        self.transit_records.splice(0..0, new_records);
                        let in_detail = matches!(self.view, View::TransitDetail(_));
                        if !self.auto_follow || in_detail {
                            self.unseen_count += new_record_count;
                        }
                        if self.auto_follow && !in_detail {
                            self.select_first();
                        } else {
                            let current_offset = self.transit_table_state.offset();
                            *self.transit_table_state.offset_mut() =
                                current_offset + new_record_count;
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
                PollMessage::OlderPage(page) => {
                    self.is_loading_older = false;
                    self.has_older_records = page.has_more;
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
                        self.oldest_transit_stored_at = new_records.last().map(|r| r.stored_at);
                        self.transit_records.extend(new_records);
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
                    self.is_loading_newer = false;
                    self.is_loading_older = false;
                    tracing::error!("{}", err);
                }
            }
        }
    }

    fn select_first(&mut self) {
        self.unseen_count = 0;
        if self.transit_records.is_empty() {
            self.transit_table_state.select(None);
        } else {
            self.transit_table_state.select(Some(0));
        }
    }

    fn request_older_records_if_needed(&mut self) {
        let selected = self.transit_table_state.selected().unwrap_or(0);
        let at_end = !self.transit_records.is_empty()
            && selected >= self.transit_records.len().saturating_sub(1);

        if !at_end || !self.has_older_records || self.is_loading_older {
            return;
        }
        self.is_loading_older = true;

        let cursor = self
            .oldest_transit_stored_at
            .or_else(|| self.transit_records.last().map(|r| r.stored_at));

        let storages = self.storages.clone();
        let tx = self.poll_tx.clone();
        let handle = tokio::runtime::Handle::current();
        handle.spawn(async move {
            let limit = NonZeroU32::new(OLDER_PAGE_SIZE).unwrap();
            let result = storages
                .transit
                .list_transits(TransitQuery {
                    cursor,
                    direction: Direction::Before,
                    limit,
                })
                .await;
            match result {
                Ok(page) => {
                    let _ = tx.send(PollMessage::OlderPage(page));
                }
                Err(e) => {
                    let _ = tx.send(PollMessage::Error(e.to_string()));
                }
            }
        });
    }
}

const PG_BUTTON_JUMP: usize = 10;
const FILE_LABEL_WIDTH: u16 = 6; // " File "
const HELP_LABEL_WIDTH: u16 = 6; // " Help "
const INITIAL_PAGE_SIZE: u32 = 50;
const OLDER_PAGE_SIZE: u32 = 25;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

enum PollMessage {
    Loading,
    Page(TransitPage),
    OlderPage(TransitPage),
    Backfill(Vec<TransitRecord>),
    Error(String),
}

async fn poll_loop(storages: Storages, tx: mpsc::Sender<PollMessage>) {
    let limit = NonZeroU32::new(INITIAL_PAGE_SIZE).unwrap();
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
