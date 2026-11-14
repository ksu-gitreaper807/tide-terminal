use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionRange, SelectionType},
    term::{self, TermMode, cell::Flags},
    vte::ansi::{Color, NamedColor},
};
use crossbeam_channel::{Receiver, Sender, bounded};

#[derive(Clone)]
pub struct Listener(Sender<Event>);
impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.0.try_send(event);
    }
}
pub struct Size {
    pub rows: usize,
    pub cols: usize,
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub type Core = Term<Listener>;
pub fn create(rows: usize, cols: usize) -> (Core, Receiver<Event>) {
    let (tx, rx) = bounded(128);
    (
        Term::new(
            term::Config {
                scrolling_history: 10_000,
                osc52: term::Osc52::Disabled,
                ..Default::default()
            },
            &Size { rows, cols },
            Listener(tx),
        ),
        rx,
    )
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub row: usize,
    pub col: usize,
    pub text: String,
    pub fg: [u8; 3],
    pub bg: Option<[u8; 3]>,
    pub wide: bool,
    pub underline: bool,
    pub strike: bool,
}
#[derive(Clone, Debug)]
pub struct View {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Cell>,
    pub cursor: (i32, usize),
    pub cursor_shape: alacritty_terminal::vte::ansi::CursorShape,
    pub mode: TermMode,
    pub offset: usize,
    pub selection: Option<SelectionRange>,
}

pub const PALETTE: [[u8; 3]; 16] = [
    [24, 29, 38],
    [236, 118, 125],
    [158, 206, 143],
    [234, 204, 139],
    [129, 169, 226],
    [186, 154, 224],
    [114, 203, 198],
    [213, 220, 229],
    [100, 114, 133],
    [245, 146, 150],
    [183, 223, 165],
    [245, 217, 159],
    [157, 191, 237],
    [204, 177, 237],
    [146, 220, 214],
    [241, 245, 249],
];
pub fn color(c: Color) -> [u8; 3] {
    match c {
        Color::Spec(c) => [c.r, c.g, c.b],
        Color::Named(NamedColor::Background) => [15, 20, 28],
        Color::Named(NamedColor::Foreground | NamedColor::BrightForeground) => [213, 220, 229],
        Color::Named(NamedColor::Cursor) => [114, 203, 198],
        Color::Named(n) => {
            let i = n as usize;
            if i < 16 {
                PALETTE[i]
            } else if (259..267).contains(&i) {
                PALETTE[i - 259].map(|c| (c as f32 * 0.65) as u8)
            } else {
                [145, 156, 172]
            }
        }
        Color::Indexed(i) if i < 16 => PALETTE[i as usize],
        Color::Indexed(i) if i >= 232 => {
            let c = 8 + (i - 232) * 10;
            [c, c, c]
        }
        Color::Indexed(i) => {
            let n = i - 16;
            let step = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
            [step(n / 36), step(n / 6 % 6), step(n % 6)]
        }
    }
}
pub fn snapshot(term: &Core) -> View {
    let offset = term.grid().display_offset();
    let mut cells = Vec::new();
    for cell in term.grid().display_iter() {
        let row = cell.point.line.0 + offset as i32;
        if row < 0 || row >= term.screen_lines() as i32 {
            continue;
        }
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let mut fg = color(cell.fg);
        let mut bg = if cell.bg == Color::Named(NamedColor::Background) {
            None
        } else {
            Some(color(cell.bg))
        };
        if cell.flags.contains(Flags::INVERSE) {
            let old = fg;
            fg = bg.unwrap_or([15, 20, 28]);
            bg = Some(old);
        }
        if cell.flags.contains(Flags::DIM) {
            fg = fg.map(|c| (c as f32 * 0.65) as u8);
        }
        // Alacritty retains tab markers for text selection. They occupy blank cells on
        // screen; handing a literal tab to the glyph renderer makes spacing depend on
        // how the PTY read happened to be split.
        let mut text = if cell.c == '\t' { ' ' } else { cell.c }.to_string();
        if let Some(chars) = cell.zerowidth() {
            text.extend(chars);
        }
        if cell.flags.contains(Flags::HIDDEN) {
            text = " ".into();
        }
        if cell.c != ' '
            || bg.is_some()
            || cell
                .flags
                .intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
        {
            cells.push(Cell {
                row: row as usize,
                col: cell.point.column.0,
                text,
                fg,
                bg,
                wide: cell.flags.contains(Flags::WIDE_CHAR),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                strike: cell.flags.contains(Flags::STRIKEOUT),
            });
        }
    }
    let cursor = term.grid().cursor.point;
    View {
        rows: term.screen_lines(),
        cols: term.columns(),
        cells,
        cursor: (cursor.line.0 + offset as i32, cursor.column.0),
        cursor_shape: term.cursor_style().shape,
        mode: *term.mode(),
        offset,
        selection: term.selection.as_ref().and_then(|s| s.to_range(term)),
    }
}
pub fn scroll(term: &mut Core, delta: i32) {
    term.scroll_display(Scroll::Delta(delta));
}
fn selection_point(term: &Core, visible: (usize, usize)) -> Point {
    Point::new(
        Line(visible.0.min(term.screen_lines() - 1) as i32 - term.grid().display_offset() as i32),
        Column(visible.1.min(term.columns() - 1)),
    )
}
/// Anchor to the grid's retained content rather than to a viewport row.
pub fn begin_selection(term: &mut Core, visible: (usize, usize)) {
    let mut selection = Selection::new(
        SelectionType::Simple,
        selection_point(term, visible),
        Side::Left,
    );
    selection.include_all();
    term.selection = Some(selection);
}
pub fn extend_selection(term: &mut Core, visible: (usize, usize)) {
    let point = selection_point(term, visible);
    if let Some(selection) = &mut term.selection {
        selection.update(point, Side::Left);
        selection.include_all();
    }
}
pub fn selected(term: &Core, start: (usize, usize), end: (usize, usize)) -> String {
    let offset = term.grid().display_offset() as i32;
    let (a, b) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    term.bounds_to_string(
        Point::new(
            Line(a.0 as i32 - offset),
            Column(a.1.min(term.columns() - 1)),
        ),
        Point::new(
            Line(b.0 as i32 - offset),
            Column(b.1.min(term.columns() - 1)),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vt_resize_alt_screen_and_wide_cells() {
        let (mut t, _) = create(5, 20);
        let mut p: alacritty_terminal::vte::ansi::Processor =
            alacritty_terminal::vte::ansi::Processor::new();
        p.advance(&mut t, "\x1b[31mred\x1b[0m 日本 e\u{301}".as_bytes());
        let v = snapshot(&t);
        assert_eq!(v.cells[0].fg, PALETTE[1]);
        assert!(v.cells.iter().any(|c| c.wide));
        assert!(v.cells.iter().any(|c| c.text == "e\u{301}"));
        p.advance(&mut t, b"\x1b[?1049heditor");
        assert!(t.mode().contains(TermMode::ALT_SCREEN));
        p.advance(&mut t, b"\x1b[?1049l");
        assert!(selected(&t, (0, 0), (0, 2)).contains("red"));
        t.resize(Size { rows: 8, cols: 40 });
        assert_eq!((t.screen_lines(), t.columns()), (8, 40));
    }
}
