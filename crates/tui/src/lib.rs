use std::io;

use conduit_core::{Config, Storages};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};

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
    _storages: Storages,
    should_quit: bool,
    color: Color,
    button_area: Rect,
}

impl App {
    fn new(config: Config, storages: Storages) -> Self {
        Self {
            _config: config,
            _storages: storages,
            should_quit: false,
            color: random_color(),
            button_area: Rect::default(),
        }
    }

    fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let [area] = Layout::horizontal([Constraint::Max(40)])
            .flex(Flex::Center)
            .areas(frame.area());

        let [title_area, button_area] = Layout::vertical([Constraint::Max(5), Constraint::Max(3)])
            .flex(Flex::Center)
            .areas(area);

        let block = Block::default()
            .title(" ⚡️ Conduit Dashboard ")
            .borders(Borders::ALL)
            .style(Style::default().fg(self.color));
        let paragraph = Paragraph::new("Press 'q' to quit.").centered().block(block);
        frame.render_widget(paragraph, title_area);

        let button = Paragraph::new("[ CHANGE COLOR ]").centered().block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(self.color)),
        );
        frame.render_widget(button, button_area);
        self.button_area = button_area;
    }

    fn handle_events(&mut self) -> anyhow::Result<()> {
        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                    KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        self.should_quit = true;
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        self.color = random_color();
                    }
                    _ => {}
                },
                Event::Mouse(mouse) => {
                    if matches!(mouse.kind, MouseEventKind::Down(_))
                        && self.button_area.contains((mouse.column, mouse.row).into())
                    {
                        self.color = random_color();
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn random_color() -> Color {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let mut x = seed.wrapping_add(1);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    let bytes = x.to_le_bytes();
    Color::Rgb(bytes[0], bytes[1], bytes[2])
}
