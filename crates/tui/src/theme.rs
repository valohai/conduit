use ratatui::layout::Alignment;
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, Default)]
pub enum MenubarDecoration {
    #[default]
    None,
    /// Banana ) that oscillates back and forth
    Banana,
    /// Ouroboros snake rotating using block characters
    Snake,
}

/// Width of the banana animation field (in columns).
const BANANA_WIDTH: usize = 9;

impl MenubarDecoration {
    /// Returns (decoration_string, display_width) for the given tick (each tick ~100ms).
    pub fn frame(self, tick: u64) -> (String, u16) {
        match self {
            MenubarDecoration::None => (String::new(), 0),
            MenubarDecoration::Banana => {
                // Banana oscillating: ) flying right, ( flying left
                let cycle = BANANA_WIDTH * 2 - 2; // positions before repeating
                let pos = (tick / 2) as usize % cycle;
                let (col, ch) = if pos < BANANA_WIDTH {
                    // Flying left: ) at position (WIDTH-1 - pos)
                    (BANANA_WIDTH - 1 - pos, ')')
                } else {
                    // Flying right: ( at position (pos - WIDTH)
                    (pos - BANANA_WIDTH + 1, '(')
                };
                let mut buf = String::with_capacity(BANANA_WIDTH);
                for i in 0..BANANA_WIDTH {
                    buf.push(if i == col { ch } else { ' ' });
                }
                (buf, BANANA_WIDTH as u16)
            }
            MenubarDecoration::Snake => {
                // Ouroboros: snake rotating around a 4-char-wide rectangle
                // using upper/lower half-block characters.
                // 8 positions around the perimeter (4 top + 4 bottom):
                //   top L→R: 0 1 2 3
                //   bot R→L: 4 5 6 7
                const WIDTH: usize = 6;
                const LOOP_LEN: usize = WIDTH * 2;
                const SNAKE_LEN: usize = 5;

                let head = (tick / 2) as usize % LOOP_LEN;

                let mut top = [false; WIDTH];
                let mut bot = [false; WIDTH];
                for i in 0..SNAKE_LEN {
                    let pos = (head + LOOP_LEN - i) % LOOP_LEN;
                    if pos < WIDTH {
                        top[pos] = true;
                    } else {
                        bot[LOOP_LEN - 1 - pos] = true;
                    }
                }

                let mut s = String::with_capacity(WIDTH);
                for i in 0..WIDTH {
                    s.push(match (top[i], bot[i]) {
                        (true, true) => '\u{2588}',  // █ full block
                        (true, false) => '\u{2580}', // ▀ upper half
                        (false, true) => '\u{2584}', // ▄ lower half
                        (false, false) => ' ',
                    });
                }
                (s, WIDTH as u16)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum ThemeName {
    #[default]
    Default,
    QBasic,
    Gorillas,
    Nibbles,
}

impl ThemeName {
    pub fn theme(self) -> Theme {
        match self {
            ThemeName::Default => Theme::default_theme(),
            ThemeName::QBasic => Theme::qbasic(),
            ThemeName::Gorillas => Theme::gorillas(),
            ThemeName::Nibbles => Theme::nibbles(),
        }
    }
}

impl std::str::FromStr for ThemeName {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "default" => Ok(ThemeName::Default),
            "qbasic" => Ok(ThemeName::QBasic),
            "gorillas" => Ok(ThemeName::Gorillas),
            "nibbles" => Ok(ThemeName::Nibbles),
            _ => Err(format!("unknown theme: {s}")),
        }
    }
}

impl std::fmt::Display for ThemeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeName::Default => write!(f, "default"),
            ThemeName::QBasic => write!(f, "qbasic"),
            ThemeName::Gorillas => write!(f, "gorillas"),
            ThemeName::Nibbles => write!(f, "nibbles"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    /// Base style applied to the entire frame (background color etc.)
    pub base: Style,
    /// Table header row
    pub table_header: Style,
    /// Selected/highlighted row
    pub row_highlight: Style,
    /// Block borders
    pub border: Style,
    /// Block titles
    pub title: Style,
    /// Title alignment
    pub title_alignment: Alignment,
    /// Status / help text at the bottom
    pub status: Style,
    /// "Loading..." text
    pub loading: Style,
    /// Unseen-records badge
    pub unseen_badge: Style,
    /// Format string for the unseen badge ("{}" is replaced with the count)
    pub unseen_badge_fmt: &'static str,
    /// Detail view labels
    pub label: Style,

    // Log panel
    pub log_error: Style,
    pub log_warn: Style,
    pub log_info: Style,
    pub log_debug: Style,
    pub log_other: Style,
    pub log_close_normal: Style,
    pub log_close_hover: Style,

    // Menu bar
    /// Menu bar background + text. None = no menu bar.
    pub menubar: Option<Style>,
    /// Highlighted/selected menu item
    pub menubar_selected: Style,
    /// Menu item hotkey letter (the underlined letter)
    pub menubar_hotkey: Style,
    /// Dropdown menu background + text
    pub menu_dropdown: Style,
    /// Dropdown selected item
    pub menu_dropdown_selected: Style,
    /// Animated menubar decoration kind
    pub menubar_decoration: MenubarDecoration,
    /// Style for the menubar decoration
    pub menubar_decoration_style: Style,

    /// Data-bar background for numeric cells (Excel-style proportional fill)
    pub data_bar: Style,

    // JSON highlighting
    pub json_key: Style,
    pub json_string: Style,
    pub json_number: Style,
    pub json_bool_null: Style,
    pub json_punct: Style,
}

impl Theme {
    pub fn default_theme() -> Self {
        Self {
            base: Style::default(),
            table_header: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            row_highlight: Style::default().add_modifier(Modifier::REVERSED),
            border: Style::default(),
            title: Style::default(),
            title_alignment: Alignment::Left,
            status: Style::default().fg(Color::DarkGray),
            loading: Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
            unseen_badge: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            unseen_badge_fmt: "\u{2191} {} new (press f) \u{2191} ",
            label: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),

            log_error: Style::default().fg(Color::Red),
            log_warn: Style::default().fg(Color::Yellow),
            log_info: Style::default().fg(Color::LightBlue),
            log_debug: Style::default().fg(Color::Gray),
            log_other: Style::default().fg(Color::DarkGray),
            log_close_normal: Style::default().fg(Color::DarkGray),
            log_close_hover: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),

            menubar: None,
            menubar_selected: Style::default(),
            menubar_hotkey: Style::default(),
            menu_dropdown: Style::default(),
            menu_dropdown_selected: Style::default(),
            menubar_decoration: MenubarDecoration::None,
            menubar_decoration_style: Style::default(),

            data_bar: Style::default().bg(Color::Rgb(0x2A, 0x5A, 0x8C)),

            json_key: Style::new().fg(Color::LightCyan),
            json_string: Style::new().fg(Color::Green),
            json_number: Style::new().fg(Color::Yellow),
            json_bool_null: Style::new().fg(Color::Magenta),
            json_punct: Style::new().fg(Color::DarkGray),
        }
    }

    pub fn qbasic() -> Self {
        // Classic QBasic IDE: blue background, bright text
        let bg = Color::Rgb(0x00, 0x00, 0xA3);
        let fg = Color::Rgb(0xAA, 0xAA, 0xAA);
        let status_bg = Color::Rgb(0x4C, 0xA7, 0xA9);
        Self {
            base: Style::default().bg(bg).fg(fg),
            table_header: Style::default()
                .bg(bg)
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            row_highlight: Style::default()
                .bg(status_bg)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
            border: Style::default().bg(bg).fg(fg),
            title: Style::default()
                .bg(Color::White)
                .fg(bg)
                .add_modifier(Modifier::BOLD),
            title_alignment: Alignment::Center,
            status: Style::default().bg(status_bg).fg(Color::Black),
            loading: Style::default()
                .bg(bg)
                .fg(Color::LightYellow)
                .add_modifier(Modifier::ITALIC),
            unseen_badge: Style::default()
                .bg(bg)
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
            unseen_badge_fmt: "\u{2191} {} new (press f) \u{2191} ",
            label: Style::default()
                .bg(bg)
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),

            log_error: Style::default().bg(bg).fg(Color::LightRed),
            log_warn: Style::default().bg(bg).fg(Color::Yellow),
            log_info: Style::default().bg(bg).fg(Color::LightCyan),
            log_debug: Style::default().bg(bg).fg(Color::Gray),
            log_other: Style::default().bg(bg).fg(Color::White),
            log_close_normal: Style::default().bg(bg).fg(Color::Gray),
            log_close_hover: Style::default()
                .bg(bg)
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),

            menubar: Some(Style::default().bg(fg).fg(Color::Black)),
            menubar_selected: Style::default().bg(Color::Black).fg(Color::White),
            menubar_hotkey: Style::default()
                .bg(fg)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            menu_dropdown: Style::default().bg(fg).fg(Color::Black),
            menu_dropdown_selected: Style::default().bg(Color::Black).fg(Color::White),
            menubar_decoration: MenubarDecoration::None,
            menubar_decoration_style: Style::default(),

            data_bar: Style::default().bg(Color::Rgb(0x20, 0x60, 0xA0)),

            json_key: Style::new().bg(bg).fg(Color::LightCyan),
            json_string: Style::new().bg(bg).fg(Color::LightGreen),
            json_number: Style::new().bg(bg).fg(Color::LightYellow),
            json_bool_null: Style::new().bg(bg).fg(Color::LightMagenta),
            json_punct: Style::new().bg(bg).fg(Color::Gray),
        }
    }

    pub fn gorillas() -> Self {
        // GORILLAS.BAS: dark slate sky, banana-yellow accents, )
        let bg = Color::Rgb(0x22, 0x2F, 0x3E);
        let fg = Color::Rgb(0xAA, 0xAA, 0xAA);
        let menu_bg = Color::Rgb(0x3B, 0x3B, 0x98);
        Self {
            base: Style::default().bg(bg).fg(fg),
            table_header: Style::default()
                .bg(bg)
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
            row_highlight: Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
            border: Style::default().bg(bg).fg(Color::LightYellow),
            title: Style::default()
                .bg(Color::Yellow)
                .fg(bg)
                .add_modifier(Modifier::BOLD),
            title_alignment: Alignment::Center,
            status: Style::default().bg(Color::Yellow).fg(Color::Black),
            loading: Style::default()
                .bg(bg)
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
            unseen_badge: Style::default()
                .bg(bg)
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
            // The banana ) — iconic gorilla projectile
            unseen_badge_fmt: ") {} new (press f) ) ",
            label: Style::default()
                .bg(bg)
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),

            log_error: Style::default().bg(bg).fg(Color::LightRed),
            log_warn: Style::default().bg(bg).fg(Color::Yellow),
            log_info: Style::default().bg(bg).fg(Color::LightCyan),
            log_debug: Style::default().bg(bg).fg(Color::Gray),
            log_other: Style::default().bg(bg).fg(Color::White),
            log_close_normal: Style::default().bg(bg).fg(Color::Gray),
            log_close_hover: Style::default()
                .bg(bg)
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),

            menubar: Some(Style::default().bg(menu_bg).fg(Color::White)),
            menubar_selected: Style::default().bg(Color::Black).fg(Color::Yellow),
            menubar_hotkey: Style::default()
                .bg(menu_bg)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            menu_dropdown: Style::default().bg(menu_bg).fg(Color::White),
            menu_dropdown_selected: Style::default().bg(Color::Black).fg(Color::Yellow),
            menubar_decoration: MenubarDecoration::Banana,
            menubar_decoration_style: Style::default()
                .bg(menu_bg)
                .fg(Color::Rgb(0xFF, 0xD3, 0x2A)),

            data_bar: Style::default().bg(Color::Rgb(0x7A, 0x6A, 0x10)),

            json_key: Style::new().bg(bg).fg(Color::LightCyan),
            json_string: Style::new().bg(bg).fg(Color::LightYellow),
            json_number: Style::new().bg(bg).fg(Color::Yellow),
            json_bool_null: Style::new().bg(bg).fg(Color::LightMagenta),
            json_punct: Style::new().bg(bg).fg(Color::White),
        }
    }

    pub fn nibbles() -> Self {
        // NIBBLES.BAS: dark background, green snake, food pellets
        let bg = Color::Black;
        Self {
            base: Style::default().bg(bg).fg(Color::Gray),
            table_header: Style::default()
                .bg(bg)
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
            row_highlight: Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
            border: Style::default().bg(bg).fg(Color::Green),
            title: Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
            title_alignment: Alignment::Center,
            status: Style::default().bg(Color::Green).fg(Color::Black),
            loading: Style::default()
                .bg(bg)
                .fg(Color::Green)
                .add_modifier(Modifier::ITALIC),
            unseen_badge: Style::default()
                .bg(bg)
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
            // Snake-like arrow wiggle
            unseen_badge_fmt: "~ {} new (press f) ~ ",
            label: Style::default()
                .bg(bg)
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),

            log_error: Style::default().bg(bg).fg(Color::LightRed),
            log_warn: Style::default().bg(bg).fg(Color::Yellow),
            log_info: Style::default().bg(bg).fg(Color::LightGreen),
            log_debug: Style::default().bg(bg).fg(Color::DarkGray),
            log_other: Style::default().bg(bg).fg(Color::Gray),
            log_close_normal: Style::default().bg(bg).fg(Color::DarkGray),
            log_close_hover: Style::default()
                .bg(bg)
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),

            menubar: Some(Style::default().bg(Color::Gray).fg(Color::Black)),
            menubar_selected: Style::default().bg(Color::Black).fg(Color::LightGreen),
            menubar_hotkey: Style::default()
                .bg(Color::Gray)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            menu_dropdown: Style::default().bg(Color::Gray).fg(Color::Black),
            menu_dropdown_selected: Style::default().bg(Color::Black).fg(Color::LightGreen),
            menubar_decoration: MenubarDecoration::Snake,
            menubar_decoration_style: Style::default()
                .bg(Color::Gray)
                .fg(Color::Rgb(0x0E, 0x27, 0x05)),

            data_bar: Style::default().bg(Color::Rgb(0x1A, 0x50, 0x1A)),

            json_key: Style::new().bg(bg).fg(Color::LightGreen),
            json_string: Style::new().bg(bg).fg(Color::Green),
            json_number: Style::new().bg(bg).fg(Color::LightYellow),
            json_bool_null: Style::new().bg(bg).fg(Color::LightCyan),
            json_punct: Style::new().bg(bg).fg(Color::DarkGray),
        }
    }
}
