use nix::libc::STDIN_FILENO;
use nix::sys::termios;
use nix::unistd;
use std::io::{stdout, Write as _};

#[derive(Debug, Clone, Copy)]
enum Event {
    KeyEscape,
    KeyTab,
    KeyBackspace,
    KeyDelete,
    KeyReturn,
    KeyUp,
    KeyDown,
    KeyLeft,
    KeyRight,
    Ctrl(char),
    Char(char),
}

#[derive(Debug, Clone, Copy)]
enum Command {
    CursorPrevChar,
    CursorPrevCharMatch(char),
    CursorNextChar,
    CursorNextCharMatch(char),
    CursorPrevWordHead,
    CursorPrevWordHeadWide,
    CursorNextWordHead,
    CursorNextWordHeadWide,
    CursorNextWordEnd,
    CursorNextWordEndWide,
    CursorEnd,
    CursorBegin,
    CursorVeryBegin,
    HistoryPrev,
    HistoryNext,
    DeletePrevChar,
    DeleteNextChar,
    DeletePrevWord,
    DeleteLine,
    DeleteRange { origin: usize },
    Commit,
    ChangeModeToInsert,
    ChangeModeToNormal,
    ChangeModeToVisualChar,
    ChangeModeToVisualLine,
    Insert(char),
}

pub enum EditError {
    Aborted,
    Exitted,
}

pub struct LineEditor {
    mode: Mode,

    history: Vec<Line>,
    temporal: Vec<Line>,
    row: isize,
}

impl Drop for LineEditor {
    fn drop(&mut self) {
        // TODO: save `self.history` to a file
    }
}

impl LineEditor {
    pub fn new() -> Self {
        Self {
            mode: Mode::Insert(InsertMode::default()),

            // TODO: restore saved history
            history: Vec::new(),

            temporal: Vec::new(),
            row: 0,
        }
    }

    pub fn read_line(&mut self) -> Result<String, EditError> {
        let saved_termios = enable_raw_mode();

        let _defer = Defer::new(|| {
            let now = termios::SetArg::TCSANOW;
            let _ = termios::tcsetattr(STDIN_FILENO, now, &saved_termios);

            print!("\x1b[2 q"); // block cursor
            stdout().flush().unwrap();
        });

        macro_rules! current_line {
            () => {{
                let len = self.temporal.len() as isize;
                self.temporal
                    .get_mut((len - 1 + self.row) as usize)
                    .unwrap()
            }};
        }

        self.new_line();

        // Save cursor
        print!("\x1b7");
        stdout().flush().unwrap();

        let mut read_buf = vec![0_u8; 32];
        'edit: loop {
            {
                // TODO: support multi-line editing

                let line = current_line!();

                // Restore cursor
                print!("\x1b8");

                // Erase lines
                print!("\x1b[J");

                let hl_range = {
                    if let Mode::Visual(VisualMode { origin }) = self.mode {
                        if origin == isize::MIN {
                            Some((0, line.buf.len()))
                        } else {
                            let mut i = origin as usize;
                            let mut j = line.cursor;
                            if i > j {
                                std::mem::swap(&mut i, &mut j);
                            }
                            Some((i, j + 1))
                        }
                    } else {
                        None
                    }
                };

                for (i, (ch, _)) in line.buf.iter().enumerate() {
                    let mut highlight = false;
                    if let Some(hl) = hl_range {
                        if hl.0 <= i && i < hl.1 {
                            highlight = true;
                        }
                    }

                    if highlight {
                        print!("\x1b[42m{ch}\x1b[m");
                    } else {
                        print!("{ch}");
                    }
                }

                print!("\x1b8");
                let cursor_step = line.buf.iter().take(line.cursor).fold(0, |a, (_, w)| a + w);
                if cursor_step > 0 {
                    print!("\x1b[{}C", cursor_step);
                }

                // change cursor shape
                if matches!(self.mode, Mode::Insert(..)) {
                    print!("\x1b[6 q"); // bar cursor
                } else {
                    print!("\x1b[2 q"); // block cursor
                }

                stdout().flush().unwrap();
            }

            let nb = unistd::read(STDIN_FILENO, &mut read_buf[..]).expect("read STDIN");
            let input = &read_buf[..nb];

            let mut event = Vec::new();

            // TODO: implement a parser
            if let Ok(input) = std::str::from_utf8(input) {
                if input == "\x1b[D" {
                    event.push(Event::KeyLeft);
                } else if input == "\x1b[C" {
                    event.push(Event::KeyRight);
                } else if input == "\x1b[A" {
                    event.push(Event::KeyUp);
                } else if input == "\x1b[B" {
                    event.push(Event::KeyDown);
                } else if input == "\x1b[3~" {
                    event.push(Event::KeyDelete);
                } else {
                    for ch in input.chars() {
                        match ch {
                            '\x03' => event.push(Event::Ctrl('c')),
                            '\x04' => event.push(Event::Ctrl('d')),
                            '\x0d' => event.push(Event::KeyReturn),
                            '\x09' => event.push(Event::KeyTab),
                            '\x1b' => event.push(Event::KeyEscape),
                            '\x17' => event.push(Event::Ctrl('w')),
                            '\x7f' => event.push(Event::KeyBackspace),
                            _ => event.push(Event::Char(ch)),
                        }
                    }
                }
            }

            let mut commands = Vec::new();
            for ev in event {
                match (&mut self.mode, ev) {
                    (_, Event::Ctrl('c')) => return Err(EditError::Aborted),
                    (_, Event::Ctrl('d')) => return Err(EditError::Exitted),
                    (Mode::Insert(state), ev) => state.process_event(ev, &mut commands),
                    (Mode::Normal(state), ev) => state.process_event(ev, &mut commands),
                    (Mode::Visual(state), ev) => state.process_event(ev, &mut commands),
                }
            }

            for cmd in commands {
                match cmd {
                    Command::ChangeModeToNormal => {
                        self.mode = Mode::Normal(NormalMode::default());
                    }
                    Command::ChangeModeToInsert => {
                        self.mode = Mode::Insert(InsertMode::default());
                    }
                    Command::ChangeModeToVisualChar => {
                        let cursor = current_line!().cursor;
                        self.mode = Mode::Visual(VisualMode::new_char(cursor));
                    }
                    Command::ChangeModeToVisualLine => {
                        self.mode = Mode::Visual(VisualMode::new_line());
                    }

                    Command::HistoryPrev => {
                        let new_row = self.row - 1;
                        if self.temporal.len() as isize - 1 + new_row >= 0 {
                            self.row = new_row;
                            current_line!().cursor_end_of_line();
                        } else {
                            // copy from history
                            let i = self.history.len() as isize + new_row;
                            if i >= 0 {
                                let picked_line = self.history[i as usize].clone();
                                self.temporal.insert(0, picked_line);
                                self.row = new_row;
                                current_line!().cursor_end_of_line();
                            }
                        }
                    }
                    Command::HistoryNext => {
                        if self.row < 0 {
                            self.row += 1;
                            current_line!().cursor_end_of_line();
                        }
                    }

                    Command::CursorPrevChar => current_line!().cursor_prev_char(),
                    Command::CursorNextChar => current_line!().cursor_next_char(),
                    Command::CursorPrevCharMatch(ch) => {
                        current_line!().cursor_prev_char_match(ch);
                    }
                    Command::CursorNextCharMatch(ch) => {
                        current_line!().cursor_next_char_match(ch);
                    }

                    Command::CursorPrevWordHead => current_line!().cursor_prev_word_head(false),
                    Command::CursorPrevWordHeadWide => {
                        current_line!().cursor_prev_word_head(true);
                    }
                    Command::CursorNextWordHead => current_line!().cursor_next_word_head(false),
                    Command::CursorNextWordHeadWide => {
                        current_line!().cursor_next_word_head(true);
                    }
                    Command::CursorNextWordEnd => current_line!().cursor_next_word_end(false),
                    Command::CursorNextWordEndWide => {
                        current_line!().cursor_next_word_end(true);
                    }
                    Command::CursorEnd => {
                        current_line!().cursor_end_of_line();
                    }
                    Command::CursorBegin => {
                        current_line!().cursor_begin_of_line();
                    }
                    Command::CursorVeryBegin => {
                        current_line!().cursor_very_begin_of_line();
                    }

                    Command::Insert(ch) => current_line!().insert(ch),

                    Command::DeletePrevChar => current_line!().delete_prev(),
                    Command::DeleteNextChar => current_line!().delete_next(),
                    Command::DeletePrevWord => current_line!().delete_word(),
                    Command::DeleteLine => current_line!().delete_line(),

                    Command::DeleteRange { origin } => {
                        let mut i = origin as usize;
                        let mut j = current_line!().cursor;
                        if i > j {
                            std::mem::swap(&mut i, &mut j);
                        }
                        current_line!().delete_range(i, j + 1);
                    }

                    Command::Commit => break 'edit,
                }

                if !self.mode.is_insert() {
                    current_line!().normal_mode_fix_cursor();
                }
            }
        }

        let line = self.commit();
        let result = line.to_string();
        self.history.push(line);
        Ok(result)
    }

    fn new_line(&mut self) {
        self.mode = Mode::Insert(InsertMode::default());
        self.temporal.clear();
        self.temporal.push(Line::new());
        self.row = 0;
    }

    fn commit(&mut self) -> Line {
        let len = self.temporal.len() as isize;
        let line = self.temporal.swap_remove((len - 1 + self.row) as usize);
        self.temporal.clear();
        self.row = 0;
        line
    }
}

#[derive(Clone)]
struct Line {
    buf: Vec<(char, usize)>,
    cursor: usize,
}

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for (ch, _) in self.buf.iter() {
            write!(f, "{}", ch)?;
        }
        Ok(())
    }
}

impl Line {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            cursor: 0,
        }
    }

    fn insert(&mut self, ch: char) {
        use unicode_width::UnicodeWidthChar as _;
        let width = ch.width().unwrap_or(1);

        self.buf.insert(self.cursor, (ch, width));
        self.cursor += 1;
    }

    fn delete_prev(&mut self) {
        if self.cursor > 0 {
            self.buf.remove(self.cursor - 1);
            self.cursor -= 1;
        }
    }

    fn delete_next(&mut self) {
        if self.cursor < self.buf.len() {
            self.buf.remove(self.cursor);
        }
    }

    fn delete_word(&mut self) {
        // remove trailing whitespaces
        while self.cursor > 0 {
            if !self.buf[self.cursor - 1].0.is_whitespace() {
                break;
            }
            self.delete_prev();
        }

        // remove a single word
        while self.cursor > 0 {
            if self.buf[self.cursor - 1].0.is_whitespace() {
                break;
            }
            self.delete_prev();
        }
    }

    fn delete_line(&mut self) {
        self.cursor = 0;
        self.buf.clear();
    }

    // delete characters in [from, to)
    fn delete_range(&mut self, from: usize, to: usize) {
        assert!(from <= to);

        let mut new_buf = Vec::new();
        for (i, (ch, sz)) in self.buf.drain(..).enumerate() {
            if from <= i && i < to {
                continue;
            }
            new_buf.push((ch, sz));
        }
        std::mem::swap(&mut self.buf, &mut new_buf);

        self.cursor = from;
    }

    fn cursor_prev_char(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn cursor_next_char(&mut self) {
        if self.cursor < self.buf.len() {
            self.cursor += 1;
        }
    }

    fn cursor_prev_char_match(&mut self, target: char) {
        let mut i = self.cursor as isize - 1;
        while i > 0 {
            if self.buf[i as usize].0 == target {
                self.cursor = i as usize;
                break;
            }
            i -= 1;
        }
    }

    fn cursor_next_char_match(&mut self, target: char) {
        let len = self.buf.len() as isize;
        let mut i = self.cursor as isize + 1;
        while i < len {
            if self.buf[i as usize].0 == target {
                self.cursor = i as usize;
                break;
            }
            i += 1;
        }
    }

    fn cursor_prev_word_head(&mut self, _wide: bool) {
        while self.cursor > 0 {
            if !self.buf[self.cursor - 1].0.is_whitespace() {
                break;
            }
            self.cursor -= 1;
        }

        while self.cursor > 0 {
            if self.buf[self.cursor - 1].0.is_whitespace() {
                break;
            }
            self.cursor -= 1;
        }
    }

    fn cursor_next_word_head(&mut self, _wide: bool) {
        let len = self.buf.len();

        while self.cursor + 1 < len {
            if self.buf[self.cursor].0.is_whitespace() {
                break;
            }
            self.cursor += 1;
        }

        while self.cursor + 1 < len {
            if !self.buf[self.cursor].0.is_whitespace() {
                break;
            }
            self.cursor += 1;
        }
    }

    fn cursor_next_word_end(&mut self, _wide: bool) {
        self.cursor_next_char();

        let len = self.buf.len();

        while self.cursor + 1 < len {
            if !self.buf[self.cursor].0.is_whitespace() {
                break;
            }
            self.cursor += 1;
        }

        while self.cursor + 1 < len {
            if self.buf[self.cursor + 1].0.is_whitespace() {
                break;
            }
            self.cursor += 1;
        }
    }

    fn cursor_end_of_line(&mut self) {
        self.cursor = self.buf.len();
    }

    fn cursor_begin_of_line(&mut self) {
        let len = self.buf.len();

        self.cursor = 0;
        while self.cursor <= len {
            if !self.buf[self.cursor].0.is_whitespace() {
                break;
            }
            self.cursor += 1;
        }
    }

    fn cursor_very_begin_of_line(&mut self) {
        self.cursor = 0;
    }

    fn normal_mode_fix_cursor(&mut self) {
        if self.cursor >= self.buf.len() {
            self.cursor = (self.buf.len() as isize - 1).max(0) as usize;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Insert(InsertMode),
    Normal(NormalMode),
    Visual(VisualMode),
}

impl Mode {
    fn is_insert(&self) -> bool {
        matches!(self, Mode::Insert(..))
    }
}

trait EditorMode {
    fn process_event(&mut self, event: Event, cmds: &mut Vec<Command>);
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct NormalMode {
    combo: String,
    last_find: Option<(char, char)>,
}

impl EditorMode for NormalMode {
    fn process_event(&mut self, event: Event, cmds: &mut Vec<Command>) {
        match self.combo.as_str() {
            "" => {
                match event {
                    Event::Char('i') => {
                        cmds.push(Command::ChangeModeToInsert);
                    }

                    Event::Char('v') => {
                        cmds.push(Command::ChangeModeToVisualChar);
                    }
                    Event::Char('V') => {
                        cmds.push(Command::ChangeModeToVisualLine);
                    }

                    Event::KeyReturn => cmds.push(Command::Commit),

                    Event::KeyLeft | Event::Char('h') => cmds.push(Command::CursorPrevChar),
                    Event::KeyRight | Event::Char('l') => cmds.push(Command::CursorNextChar),
                    Event::KeyUp | Event::Char('k') => cmds.push(Command::HistoryPrev),
                    Event::KeyDown | Event::Char('j') => cmds.push(Command::HistoryNext),

                    Event::Char('w') => cmds.push(Command::CursorNextWordHead),
                    Event::Char('W') => cmds.push(Command::CursorNextWordHeadWide),
                    Event::Char('e') => cmds.push(Command::CursorNextWordEnd),
                    Event::Char('E') => cmds.push(Command::CursorNextWordEndWide),
                    Event::Char('b') => cmds.push(Command::CursorPrevWordHead),
                    Event::Char('B') => cmds.push(Command::CursorPrevWordHeadWide),

                    Event::Char('f') => {
                        self.combo.push('f');
                    }
                    Event::Char('F') => {
                        self.combo.push('F');
                    }
                    Event::Char(';') => match self.last_find {
                        Some(('f', ch)) => {
                            cmds.push(Command::CursorNextCharMatch(ch));
                        }
                        Some(('F', ch)) => {
                            cmds.push(Command::CursorPrevCharMatch(ch));
                        }
                        _ => {}
                    },

                    Event::Char('$') => {
                        cmds.push(Command::CursorEnd);
                    }
                    Event::Char('^') => {
                        cmds.push(Command::CursorBegin);
                    }
                    Event::Char('0') => {
                        cmds.push(Command::CursorVeryBegin);
                    }
                    Event::Char('A') => {
                        cmds.push(Command::ChangeModeToInsert);
                        cmds.push(Command::CursorEnd);
                    }
                    Event::Char('I') => {
                        cmds.push(Command::ChangeModeToInsert);
                        cmds.push(Command::CursorBegin);
                    }

                    Event::Char('a') => {
                        cmds.push(Command::ChangeModeToInsert);
                        cmds.push(Command::CursorNextChar);
                    }
                    Event::Char('s') => {
                        cmds.push(Command::ChangeModeToInsert);
                        cmds.push(Command::DeleteNextChar);
                    }
                    Event::Char('x') => {
                        cmds.push(Command::DeleteNextChar);
                    }

                    Event::Char('d') => {
                        self.combo.push('d');
                    }

                    Event::Char('c') => {
                        // TODO
                    }

                    _ => {}
                }
            }

            "d" => {
                if let Event::Char('d') = event {
                    cmds.push(Command::DeleteLine);
                }
                self.combo.clear();
            }

            "f" => {
                if let Event::Char(ch) = event {
                    self.last_find = Some(('f', ch));
                    cmds.push(Command::CursorNextCharMatch(ch));
                } else {
                    self.last_find = None;
                }
                self.combo.clear();
            }
            "F" => {
                if let Event::Char(ch) = event {
                    self.last_find = Some(('F', ch));
                    cmds.push(Command::CursorPrevCharMatch(ch));
                } else {
                    self.last_find = None;
                }
                self.combo.clear();
            }

            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct InsertMode;

impl EditorMode for InsertMode {
    fn process_event(&mut self, event: Event, cmds: &mut Vec<Command>) {
        match event {
            Event::KeyEscape => {
                cmds.push(Command::CursorPrevChar);
                cmds.push(Command::ChangeModeToNormal);
            }

            Event::KeyReturn => cmds.push(Command::Commit),
            Event::KeyLeft => cmds.push(Command::CursorPrevChar),
            Event::KeyRight => cmds.push(Command::CursorNextChar),
            Event::KeyUp => cmds.push(Command::HistoryPrev),
            Event::KeyDown => cmds.push(Command::HistoryNext),

            Event::Char(ch) => cmds.push(Command::Insert(ch)),
            Event::KeyBackspace => cmds.push(Command::DeletePrevChar),
            Event::KeyDelete => cmds.push(Command::DeleteNextChar),
            Event::Ctrl('w') => cmds.push(Command::DeletePrevWord),

            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct VisualMode {
    origin: isize,
}

impl VisualMode {
    fn new_char(origin: usize) -> Self {
        Self {
            origin: origin as isize,
        }
    }

    fn new_line() -> Self {
        Self { origin: isize::MIN }
    }

    fn is_line_mode(&self) -> bool {
        self.origin == isize::MIN
    }
}

impl EditorMode for VisualMode {
    fn process_event(&mut self, event: Event, cmds: &mut Vec<Command>) {
        match event {
            Event::KeyEscape | Event::Char('v') => {
                cmds.push(Command::ChangeModeToNormal);
            }

            Event::KeyReturn => cmds.push(Command::Commit),
            Event::KeyLeft | Event::Char('h') => cmds.push(Command::CursorPrevChar),
            Event::KeyRight | Event::Char('l') => cmds.push(Command::CursorNextChar),
            Event::Char('w') => cmds.push(Command::CursorNextWordHead),
            Event::Char('W') => cmds.push(Command::CursorNextWordHeadWide),
            Event::Char('e') => cmds.push(Command::CursorNextWordEnd),
            Event::Char('E') => cmds.push(Command::CursorNextWordEndWide),
            Event::Char('b') => cmds.push(Command::CursorPrevWordHead),
            Event::Char('B') => cmds.push(Command::CursorPrevWordHeadWide),

            Event::Char('d') | Event::Char('x') => {
                if self.is_line_mode() {
                    cmds.push(Command::DeleteLine);
                } else {
                    cmds.push(Command::DeleteRange {
                        origin: self.origin as usize,
                    });
                }
                cmds.push(Command::ChangeModeToNormal);
            }
            Event::Char('c') | Event::Char('s') => {
                cmds.push(Command::ChangeModeToInsert);
                if self.is_line_mode() {
                    cmds.push(Command::DeleteLine);
                } else {
                    cmds.push(Command::DeleteRange {
                        origin: self.origin as usize,
                    });
                }
            }

            _ => {}
        }
    }
}

fn enable_raw_mode() -> termios::Termios {
    let saved = termios::tcgetattr(STDIN_FILENO).unwrap();

    let mut raw_mode = saved.clone();
    {
        use termios::ControlFlags;
        use termios::InputFlags;
        use termios::LocalFlags;
        use termios::OutputFlags;

        raw_mode.input_flags &= !(InputFlags::IGNBRK
            | InputFlags::BRKINT
            | InputFlags::PARMRK
            | InputFlags::ISTRIP
            | InputFlags::INLCR
            | InputFlags::IGNCR
            | InputFlags::ICRNL
            | InputFlags::IXON);

        raw_mode.output_flags &= !OutputFlags::OPOST;

        raw_mode.local_flags &= !(LocalFlags::ECHO
            | LocalFlags::ECHONL
            | LocalFlags::ICANON
            | LocalFlags::ISIG
            | LocalFlags::IEXTEN);

        raw_mode.control_flags &= !(ControlFlags::CSIZE | ControlFlags::PARENB);
        raw_mode.control_flags |= ControlFlags::CS8;
    }
    termios::tcsetattr(STDIN_FILENO, termios::SetArg::TCSANOW, &raw_mode).expect("tcsetattr");

    saved
}

struct Defer<F: FnOnce()> {
    f: Option<F>,
}
impl<F: FnOnce()> Defer<F> {
    fn new(f: F) -> Self {
        Self { f: Some(f) }
    }
}
impl<F: FnOnce()> Drop for Defer<F> {
    fn drop(&mut self) {
        if let Some(f) = self.f.take() {
            f();
        }
    }
}
