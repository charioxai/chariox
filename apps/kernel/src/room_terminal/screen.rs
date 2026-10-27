//! The vt100 screen model every client and agent reads.

pub(super) const SCROLLBACK_LINES: usize = 5000;

pub(super) struct Screen {
    parser: vt100::Parser,
}

pub(super) struct ScreenText {
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub screen: String,
    pub scrollback: String,
    pub ansi: Vec<u8>,
}

impl Screen {
    pub(super) fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, SCROLLBACK_LINES),
        }
    }

    /// Rebuild the screen a restarted kernel kept as text.
    pub(super) fn replay(rows: u16, cols: u16, scrollback: &str, screen: &str) -> Self {
        let mut restored = Self::new(rows, cols);
        let text = if scrollback.is_empty() {
            screen.to_string()
        } else {
            format!("{scrollback}\n{screen}")
        };
        restored.feed(text.replace('\n', "\r\n").as_bytes());
        restored
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub(super) fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
    }

    pub(super) fn visible_text(&self) -> String {
        self.parser.screen().contents()
    }

    pub(super) fn text(&mut self) -> ScreenText {
        let scrollback = self.scrollback_text();
        let screen = self.parser.screen();
        let (cursor_row, cursor_col) = screen.cursor_position();
        ScreenText {
            cursor_row,
            cursor_col,
            screen: screen.contents(),
            scrollback,
            ansi: screen.state_formatted(),
        }
    }

    /// Lines that scrolled off the top, oldest first.
    fn scrollback_text(&mut self) -> String {
        let (rows, cols) = self.parser.screen().size();
        let screen = self.parser.screen_mut();
        screen.set_scrollback(usize::MAX);
        let mut offset = screen.scrollback();
        let mut lines = Vec::new();
        while offset > 0 {
            screen.set_scrollback(offset);
            let page = usize::from(rows).min(offset);
            lines.extend(screen.rows(0, cols).take(page));
            offset -= page;
        }
        screen.set_scrollback(0);
        lines.join("\n").trim_end().to_string()
    }
}
