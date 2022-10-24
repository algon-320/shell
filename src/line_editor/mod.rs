mod line;
mod modes;
mod text_object;

use nix::libc::STDIN_FILENO;
use nix::sys::termios;
use nix::unistd;
use std::collections::HashMap;
use std::io::{stdout, Write as _};

use crate::completion;
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
    pub command_completion: Box<completion::CommandCompletion>,
}

impl Drop for LineEditor {
    fn drop(&mut self) {
        // TODO: save `self.line_history` to a file
    }
}

impl LineEditor {
    pub fn new() -> Self {
        use completion::{CommandCompletion, FileCompletion};
        let command_completion = Box::new(CommandCompletion::new(
            Vec::new(),
            Box::new(FileCompletion::new()),
        ));

        Self {
            mode: Mode::Insert(InsertMode::default()),
            registers: HashMap::new(),
            line_history: Vec::new(),
            command_completion,
        }
    }

    pub fn read_line(&mut self, prompt_prefix: String) -> Result<String, EditError> {
        let saved_termios = enable_raw_mode();

        let _defer = Defer::new(|| {
            let now = termios::SetArg::TCSANOW;
            let _ = termios::tcsetattr(STDIN_FILENO, now, &saved_termios);

            print!("\x1b[2 q"); // block cursor
            stdout().flush().unwrap();
        });

        self.new_line();

        let mut temporal: Vec<Line> = Vec::new();
        let mut row: isize = 0;
        let mut history_search_start_idx: usize = 0;

        let mut undo_stack: Vec<Line> = Vec::new();
        let mut redo_stack: Vec<Line> = Vec::new();

        {
            let line = Line::new();
            temporal.push(line.clone());

            if self.mode.is_insert() {
                undo_stack.push(line);
            }
        }

        let mut last_command = Command::Commit;
        let mut completion = CompletionEngine::new(&*self.command_completion);

        macro_rules! current_line {
            () => {{
                let len = temporal.len() as isize;
                temporal.get_mut((len - 1 + row) as usize).unwrap()
            }};
        }

        macro_rules! update_line {
            () => {{
                // TODO: support multi-line editing
                let line = current_line!();

                let color = match self.mode {
                    Mode::Insert(..) => "\x1b[36;1m",
                    Mode::Normal(..) => "\x1b[34;1m",
                    Mode::Visual(..) => "\x1b[32;1m",
                    Mode::Search(..) => "\x1b[38;5;209;1m",
                };

                let prompt_sign = if unistd::geteuid().is_root() {
                    "#"
                } else {
                    "%"
                };

                let (prompt, prompt_length) = Self::unescape_prompt(&format!(
                    "{prompt_prefix}({color}){prompt_sign}(\x1b[m) "
                ));

                print!("\x1b8"); // Restore cursor
                print!("\x1b[K"); // Erase lines
                print!("{prompt}"); // Prompt

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
                let mut line_length = prompt_length;

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
                let cursor_step =
                    prompt_length + line.iter(..).take(line.cursor()).fold(0, |a, (_, w)| a + w);
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
            }};
        }

        // Save cursor
        print!("\x1b7");
        stdout().flush().unwrap();

        let mut read_buf = vec![0_u8; 32];
        'edit: loop {
            update_line!();

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
                        let new_row = row - 1;
                        if temporal.len() as isize - 1 + new_row >= 0 {
                            row = new_row;
                            current_line!().cursor_end_of_line();
                        } else {
                            // copy from line_history
                            let i = self.line_history.len() as isize + new_row;
                            if i >= 0 {
                                let picked_line = self.line_history[i as usize].clone();
                                temporal.insert(0, picked_line);
                                row = new_row;
                                current_line!().cursor_end_of_line();
                            }
                        }
                    }
                    Command::HistoryNext => {
                        if row < 0 {
                            row += 1;
                            current_line!().cursor_end_of_line();
                        }
                    }

                    Command::HistorySearch { query, reset } => {
                        if reset {
                            history_search_start_idx = self.line_history.len() - 1;
                        }

                        let mut matched = false;
                        let idx = history_search_start_idx;

                        for (i, h) in self.line_history[0..idx].iter().enumerate().rev() {
                            let line = h.to_string();
                            if let Some(pos) = line.find(&query) {
                                row = 0;
                                *current_line!() = h.clone();
                                matched = true;
                                history_search_start_idx = i;

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
                                    row = 0;
                                    *current_line!() = h.clone();
                                    matched = true;
                                    history_search_start_idx = i;

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
                            row = 0;
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
                        undo_stack.push(current_line!().clone());
                        redo_stack.clear();
                    }
                    Command::Undo => {
                        if let Some(line) = undo_stack.pop() {
                            redo_stack.push(current_line!().clone());
                            *current_line!() = line;
                        }
                    }
                    Command::Redo => {
                        if let Some(line) = redo_stack.pop() {
                            undo_stack.push(current_line!().clone());
                            *current_line!() = line;
                        }
                    }

                    Command::TryCompleteFilename => {
                        let last_command_is_completion = last_command
                            == Command::TryCompleteFilename
                            || last_command == Command::DisplayCompletionCandidate;

                        // contiguous TryCompleteFilename would not update the candidates
                        if !last_command_is_completion || completion.cleared() {
                            // update completion candidates
                            completion.update(current_line!().to_string());
                        }

                        let line = current_line!();

                        let last_completion_len =
                            completion.prev().map(|l| l.chars().count()).unwrap_or(0);

                        for _ in 0..last_completion_len {
                            line.delete_prev();
                        }

                        if let Some(cand) = completion.next() {
                            for ch in cand.chars() {
                                current_line!().insert(ch);
                            }

                            // commit it if there is only a single choice
                            if completion.len() == 1 {
                                completion.clear();
                            }
                        }
                    }
                    Command::DisplayCompletionCandidate => {
                        // always update completion candidates
                        completion.update(current_line!().to_string());

                        if let Some(prefix) = current_line!().last_word(true) {
                            print!("\r\n\x1b[J");
                            for cand in completion.iter() {
                                print!("{prefix}{cand}\t");
                            }
                            print!("\r\n");
                            stdout().flush().unwrap();
                        }
                    }

                    Command::CdToParent => {
                        // FIXME
                        print!("\r\n\x1b[J\x1b[A");
                        stdout().flush().unwrap();
                        return Ok("cd ..".to_string());
                    }
                    Command::CdUndo => {
                        // FIXME
                        print!("\r\n\x1b[J\x1b[A");
                        stdout().flush().unwrap();
                        return Ok("cd -".to_string());
                    }
                    Command::CdRedo => {
                        // FIXME
                        print!("\r\n\x1b[J\x1b[A");
                        stdout().flush().unwrap();
                        return Ok("cd +".to_string());
                    }
                }

                if !self.mode.is_insert() {
                    current_line!().normal_mode_fix_cursor();
                }

                last_command = cmd;
            }
        }

        update_line!();

        print!("\r\n\x1b[J");
        stdout().flush().unwrap();

        let line = current_line!().clone();
        let result = line.to_string();
        if !result.is_empty() {
            self.line_history.push(line);
        }

        Ok(result)
    }

    // Returns a pair of (unescaped string, print length)
    fn unescape_prompt(prompt: &str) -> (String, usize) {
        let mut buf = String::new();
        let mut len = 0;

        let mut ignore = 0;
        let mut escaped = false;

        for ch in prompt.chars() {
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
    }

    fn new_line(&mut self) {
        let new_mode = match self.mode {
            Mode::Insert(..) | Mode::Search(..) => Mode::Insert(InsertMode::default()),
            Mode::Normal(..) | Mode::Visual(..) => Mode::Normal(NormalMode::default()),
        };
        self.mode = new_mode;
    }
}

pub struct CompletionEngine<'a> {
    completion: &'a dyn completion::Complete,
    candidates: Vec<String>,
    line: String,
    dirty: u8,
}

impl<'a> CompletionEngine<'a> {
    pub fn new(completion: &'a dyn completion::Complete) -> Self {
        Self {
            completion,
            candidates: Vec::new(),
            line: String::new(),
            dirty: 0,
        }
    }

    pub fn clear(&mut self) {
        self.candidates.clear();
        self.line.clear();
        self.dirty = 0;
    }

    pub fn cleared(&mut self) -> bool {
        self.dirty == 0
    }

    pub fn update(&mut self, line: String) {
        if self.line != line {
            let mut words: Vec<&str> = line.split_ascii_whitespace().collect();
            if line.ends_with(' ') {
                words.push("");
            }

            self.candidates = self.completion.candidates(&words);
            self.line = line;
            self.dirty = 1;
        }
    }

    pub fn next(&mut self) -> Option<&str> {
        if self.candidates.is_empty() {
            return None;
        }

        self.dirty = 2;
        let cand = self.candidates.remove(0);
        self.candidates.push(cand);
        self.candidates.last().map(String::as_str)
    }

    pub fn prev(&self) -> Option<&str> {
        if self.dirty == 2 {
            self.candidates.last().map(String::as_str)
        } else {
            None
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        self.candidates.iter().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.candidates.len()
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
