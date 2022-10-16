mod line;
mod modes;

use nix::libc::STDIN_FILENO;
use nix::sys::termios;
use nix::unistd;
use std::collections::HashMap;
use std::io::{stdout, Write as _};

use line::*;
use modes::*;

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

#[derive(Debug, Clone)]
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
    CursorExact(usize),
    HistoryPrev,
    HistoryNext,
    DeletePrevChar,
    DeleteNextChar,
    DeletePrevWord,
    DeleteLine,
    DeleteRange { from: usize, to: usize },
    Commit,
    ChangeModeToInsert,
    ChangeModeToNormal,
    ChangeModeToVisualChar,
    ChangeModeToVisualLine,
    Insert(char),
    RegisterStore { reg: char, text: String },
    RegisterPastePrev { reg: char },
    RegisterPasteNext { reg: char },
    MakeCheckPoint,
    Undo,
    Redo,
}

pub enum EditError {
    Aborted,
    Exitted,
}

pub struct LineEditor {
    mode: Mode,
    registers: HashMap<char, String>,
    line_history: Vec<Line>,
    temporal: Vec<Line>,
    row: isize,

    undo_stack: Vec<Line>,
    redo_stack: Vec<Line>,
}

impl Drop for LineEditor {
    fn drop(&mut self) {
        // TODO: save `self.line_history` to a file
    }
}

impl LineEditor {
    pub fn new() -> Self {
        Self {
            mode: Mode::Insert(InsertMode::default()),
            registers: HashMap::new(),

            // TODO: restore saved history
            line_history: Vec::new(),
            temporal: Vec::new(),
            row: 0,

            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
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

                match self.mode {
                    Mode::Insert(..) => {
                        print!("\x1b[36;1m%\x1b[m ");
                    }
                    Mode::Normal(..) => {
                        print!("\x1b[34;1m%\x1b[m ");
                    }
                    Mode::Visual(..) => {
                        print!("\x1b[32;1m%\x1b[m ");
                    }
                }

                let hl_range = {
                    if let Mode::Visual(vis_mode) = &self.mode {
                        if let Some(origin) = vis_mode.origin() {
                            let mut i = origin as usize;
                            let mut j = line.cursor();
                            if i > j {
                                std::mem::swap(&mut i, &mut j);
                            }
                            Some((i, j + 1))
                        } else {
                            Some((0, usize::MAX))
                        }
                    } else {
                        None
                    }
                };

                for (i, (ch, _)) in line.iter(..).enumerate() {
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
                let cursor_step = 2 + line.iter(..).take(line.cursor()).fold(0, |a, (_, w)| a + w);
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
                            '\x00' => event.push(Event::Ctrl('@')),
                            '\x01' => event.push(Event::Ctrl('a')),
                            '\x02' => event.push(Event::Ctrl('b')),
                            '\x03' => event.push(Event::Ctrl('c')),
                            '\x04' => event.push(Event::Ctrl('d')),
                            '\x05' => event.push(Event::Ctrl('e')),
                            '\x06' => event.push(Event::Ctrl('f')),
                            '\x07' => event.push(Event::Ctrl('g')),
                            '\x08' => event.push(Event::Ctrl('h')),
                            '\x09' => event.push(Event::KeyTab),
                            '\x0a' => event.push(Event::Ctrl('j')),
                            '\x0b' => event.push(Event::Ctrl('k')),
                            '\x0c' => event.push(Event::Ctrl('l')),
                            '\x0d' => event.push(Event::KeyReturn),
                            '\x0e' => event.push(Event::Ctrl('n')),
                            '\x0f' => event.push(Event::Ctrl('o')),
                            '\x10' => event.push(Event::Ctrl('p')),
                            '\x11' => event.push(Event::Ctrl('q')),
                            '\x12' => event.push(Event::Ctrl('r')),
                            '\x13' => event.push(Event::Ctrl('s')),
                            '\x14' => event.push(Event::Ctrl('t')),
                            '\x15' => event.push(Event::Ctrl('u')),
                            '\x16' => event.push(Event::Ctrl('v')),
                            '\x17' => event.push(Event::Ctrl('w')),
                            '\x18' => event.push(Event::Ctrl('x')),
                            '\x19' => event.push(Event::Ctrl('y')),
                            '\x1A' => event.push(Event::Ctrl('z')),
                            '\x1b' => event.push(Event::KeyEscape),
                            '\x1c' => event.push(Event::Ctrl('\\')),
                            '\x1d' => event.push(Event::Ctrl(']')),
                            '\x1e' => event.push(Event::Ctrl('^')),
                            '\x1f' => event.push(Event::Ctrl('_')),
                            '\x7f' => event.push(Event::KeyBackspace),
                            ch if ch.is_control() => {}
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
                    (Mode::Insert(state), ev) => {
                        state.process_event(ev, current_line!(), &mut commands);
                    }
                    (Mode::Normal(state), ev) => {
                        state.process_event(ev, current_line!(), &mut commands);
                    }
                    (Mode::Visual(state), ev) => {
                        state.process_event(ev, current_line!(), &mut commands);
                    }
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
                        let cursor = current_line!().cursor();
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
                            // copy from line_history
                            let i = self.line_history.len() as isize + new_row;
                            if i >= 0 {
                                let picked_line = self.line_history[i as usize].clone();
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
                    Command::CursorExact(pos) => {
                        current_line!().cursor_exact(pos);
                    }

                    Command::Insert(ch) => current_line!().insert(ch),

                    Command::DeletePrevChar => current_line!().delete_prev(),
                    Command::DeleteNextChar => current_line!().delete_next(),
                    Command::DeletePrevWord => current_line!().delete_word(),
                    Command::DeleteLine => current_line!().delete_line(),
                    Command::DeleteRange { from, to } => current_line!().delete_range(from, to),

                    Command::Commit => break 'edit,

                    Command::RegisterStore { reg, text } => {
                        self.registers.insert(reg, text);
                    }
                    Command::RegisterPastePrev { reg } => {
                        if let Some(text) = self.registers.get(&reg) {
                            let line = current_line!();
                            for ch in text.chars() {
                                line.insert(ch);
                            }
                        }
                    }
                    Command::RegisterPasteNext { reg } => {
                        if let Some(text) = self.registers.get(&reg) {
                            let line = current_line!();
                            line.cursor_next_char();
                            for ch in text.chars() {
                                line.insert(ch);
                            }
                            line.cursor_prev_char();
                        }
                    }

                    Command::MakeCheckPoint => {
                        let line = current_line!().clone();
                        self.undo_stack.push(line);
                        self.redo_stack.clear();
                    }
                    Command::Undo => {
                        if self.undo_stack.len() >= 2 {
                            let line = self.undo_stack.pop().unwrap();
                            self.redo_stack.push(line);

                            let line = self.undo_stack.last().unwrap();
                            *current_line!() = line.clone();
                        }
                    }
                    Command::Redo => {
                        if let Some(line) = self.redo_stack.pop() {
                            self.undo_stack.push(line.clone());
                            *current_line!() = line;
                        }
                    }
                }

                if !self.mode.is_insert() {
                    current_line!().normal_mode_fix_cursor();
                }
            }
        }

        let line = self.commit();
        let result = line.to_string();
        self.line_history.push(line);
        Ok(result)
    }

    fn new_line(&mut self) {
        self.mode = Mode::Insert(InsertMode::default());

        let line = Line::new();
        self.temporal.clear();
        self.temporal.push(line.clone());
        self.row = 0;

        self.undo_stack.clear();
        self.undo_stack.push(line);
        self.redo_stack.clear();
    }

    fn commit(&mut self) -> Line {
        let len = self.temporal.len() as isize;
        let line = self.temporal.swap_remove((len - 1 + self.row) as usize);
        self.temporal.clear();
        self.row = 0;
        line
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
