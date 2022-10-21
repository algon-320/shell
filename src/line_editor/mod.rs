mod completion;
mod line;
mod modes;
mod text_object;

use nix::libc::STDIN_FILENO;
use nix::sys::termios;
use nix::unistd;
use std::collections::HashMap;
use std::io::{stdout, Write as _};

use crate::terminal_size;
use line::*;
use modes::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq)]
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
    HistorySearch { query: String, reset: bool },
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
    ChangeModeToSearch,
    Insert(char),
    RegisterStore { reg: char, text: String },
    RegisterPastePrev { reg: char },
    RegisterPasteNext { reg: char },
    MakeCheckPoint,
    Undo,
    Redo,
    TryCompleteFilename,
    DisplayCompletionCandidate,
    CdToParent,
    CdUndo,
    CdRedo,
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
    history_search_start_idx: usize,

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
            history_search_start_idx: 0,

            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn read_line(&mut self, prompt_prefix: String) -> Result<String, EditError> {
        let (prompt_prefix, prompt_prefix_length) = {
            let mut buf = String::new();
            let mut len = 0;

            let mut ignore = 0;
            let mut escaped = false;

            for ch in prompt_prefix.chars() {
                if !escaped && ch == '\\' {
                    escaped = true;
                    continue;
                }

                if !escaped && ch == '(' {
                    ignore += 1;
                }

                if escaped || (ch != '(' && ch != ')') {
                    buf.push(ch);
                }

                if ignore == 0 {
                    use unicode_width::UnicodeWidthChar as _;
                    len += ch.width().unwrap_or(1);
                }

                if !escaped && ch == ')' {
                    ignore -= 1;
                }

                escaped = false;
            }

            (buf, len)
        };

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

        let file_completion = completion::FileCompletion::new_cwd();
        let mut last_candidates: Vec<String> = Vec::new();
        let mut last_completion_len: usize = 0;
        let mut last_command = Command::Commit;

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
                print!("\x1b[K");

                print!("{prompt_prefix}");
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
                    Mode::Search(..) => {
                        print!("\x1b[38;5;209;1m%\x1b[m ");
                    }
                }

                let hl_range = match &self.mode {
                    Mode::Visual(vis_mode) => {
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
                    }
                    Mode::Search(search_mode) => {
                        let query = search_mode.query();
                        // FIXME
                        let s = line.to_string();
                        if let Some(i) = s.find(&query) {
                            let from = s[..i].chars().count();
                            let len = query.chars().count();
                            let to = from + len;
                            Some((from, to))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                let terminal_width = terminal_size::get_cols() as usize;
                let mut line_length = prompt_prefix_length + 2; // FIXME

                for (i, (ch, width)) in line.iter(..).enumerate() {
                    line_length += width;
                    if line_length > terminal_width {
                        break;
                    }

                    let mut highlight = false;
                    if let Some(hl) = hl_range {
                        if hl.0 <= i && i < hl.1 {
                            highlight = true;
                        }
                    }

                    if highlight {
                        print!("\x1b[100;97m{ch}\x1b[m");
                    } else {
                        print!("{ch}");
                    }
                }

                print!("\x1b8");
                let cursor_step = prompt_prefix_length
                    + 2
                    + line.iter(..).take(line.cursor()).fold(0, |a, (_, w)| a + w);
                if cursor_step > 0 {
                    print!("\x1b[{}C", cursor_step);
                }

                // change cursor shape
                if self.mode.is_insert() {
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
                    (_, Event::Ctrl('d')) if current_line!().len() == 0 => {
                        return Err(EditError::Exitted);
                    }

                    (Mode::Insert(mode), ev) => {
                        mode.process_event(ev, current_line!(), &mut commands);
                    }
                    (Mode::Normal(mode), ev) => {
                        mode.process_event(ev, current_line!(), &mut commands);
                    }
                    (Mode::Visual(mode), ev) => {
                        mode.process_event(ev, current_line!(), &mut commands);
                    }
                    (Mode::Search(mode), ev) => {
                        mode.process_event(ev, current_line!(), &mut commands);
                    }
                }
            }

            for cmd in commands {
                match cmd.clone() {
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
                    Command::ChangeModeToSearch => {
                        self.mode = Mode::Search(SearchMode::new());
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

                    Command::HistorySearch { query, reset } => {
                        if reset {
                            self.history_search_start_idx = self.line_history.len() - 1;
                        }

                        let mut matched = false;
                        let idx = self.history_search_start_idx;

                        for (i, h) in self.line_history[0..idx].iter().enumerate().rev() {
                            let line = h.to_string();
                            if let Some(pos) = line.find(&query) {
                                self.row = 0;
                                *current_line!() = h.clone();
                                matched = true;
                                self.history_search_start_idx = i;

                                let pre = line[..pos].chars().count();
                                let len = query.chars().count();
                                current_line!().cursor_exact(pre + len);

                                break;
                            }
                        }

                        if !matched {
                            for (i, h) in self.line_history[idx..].iter().enumerate().rev() {
                                let line = h.to_string();
                                if let Some(pos) = line.find(&query) {
                                    self.row = 0;
                                    *current_line!() = h.clone();
                                    matched = true;
                                    self.history_search_start_idx = i;

                                    let pre = line[..pos].chars().count();
                                    let len = query.chars().count();
                                    current_line!().cursor_exact(pre + len);

                                    break;
                                }
                            }
                        }

                        if !matched {
                            let mut line = Line::from(query.as_str());
                            line.cursor_end_of_line();
                            self.row = 0;
                            *current_line!() = line;
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
                        self.undo_stack.push(current_line!().clone());
                        self.redo_stack.clear();
                    }
                    Command::Undo => {
                        if let Some(line) = self.undo_stack.pop() {
                            self.redo_stack.push(current_line!().clone());
                            *current_line!() = line;
                        }
                    }
                    Command::Redo => {
                        if let Some(line) = self.redo_stack.pop() {
                            self.undo_stack.push(current_line!().clone());
                            *current_line!() = line;
                        }
                    }

                    Command::TryCompleteFilename => {
                        // update completion candidates
                        if (last_command != Command::TryCompleteFilename
                            && last_command != Command::DisplayCompletionCandidate)
                            || (last_candidates.len() == 1
                                && last_candidates[0].ends_with(std::path::MAIN_SEPARATOR))
                        {
                            last_completion_len = 0;

                            if let Some(part) = current_line!().last_word(true) {
                                let cand = file_completion.candidates(&part);
                                last_candidates = cand;
                            } else {
                                last_candidates.clear();
                            }
                        }

                        let mut comp = String::new();
                        if !last_candidates.is_empty() {
                            let next = last_candidates.remove(0);
                            last_candidates.push(next.clone());
                            comp = next;
                        }

                        let line = current_line!();
                        for _ in 0..last_completion_len {
                            line.delete_prev();
                        }

                        let mut comp_len = 0;
                        for ch in comp.chars() {
                            current_line!().insert(ch);
                            comp_len += 1;
                        }

                        last_completion_len = comp_len;
                    }
                    Command::DisplayCompletionCandidate => {
                        // update completion candidates
                        {
                            last_completion_len = 0;

                            if let Some(part) = current_line!().last_word(true) {
                                let cand = file_completion.candidates(&part);
                                last_candidates = cand;
                            } else {
                                last_candidates.clear();
                            }
                        }

                        if let Some(prefix) = current_line!().last_word(true) {
                            print!("\r\n\x1b[J");
                            for cand in last_candidates.iter() {
                                print!("{prefix}{cand}\t");
                            }
                            print!("\r\n");
                            stdout().flush().unwrap();
                        }
                    }

                    Command::CdToParent => {
                        // FIXME
                        print!("\r\n\x1b[J");
                        return Ok("cd ..".to_string());
                    }
                    Command::CdUndo => {
                        // FIXME
                        print!("\r\n\x1b[J");
                        return Ok("cd -".to_string());
                    }
                    Command::CdRedo => {
                        // FIXME
                        print!("\r\n\x1b[J");
                        return Ok("cd +".to_string());
                    }
                }

                if !self.mode.is_insert() {
                    current_line!().normal_mode_fix_cursor();
                }

                last_command = cmd;
            }
        }

        print!("\r\n\x1b[J");

        let line = self.commit();
        let result = line.to_string();
        if !result.is_empty() {
            self.line_history.push(line);
        }

        Ok(result)
    }

    fn new_line(&mut self) {
        let new_mode = match self.mode {
            Mode::Insert(..) | Mode::Search(..) => Mode::Insert(InsertMode::default()),
            Mode::Normal(..) | Mode::Visual(..) => Mode::Normal(NormalMode::default()),
        };
        self.mode = new_mode;

        let line = Line::new();
        self.temporal.clear();
        self.temporal.push(line.clone());
        self.row = 0;

        self.undo_stack.clear();
        self.redo_stack.clear();

        if self.mode.is_insert() {
            self.undo_stack.push(line);
        }
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
