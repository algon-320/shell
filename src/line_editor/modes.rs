use super::*;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Mode {
    Insert(InsertMode),
    Search(SearchMode),
    Normal(NormalMode),
    Visual(VisualMode),
}

impl Mode {
    pub fn is_insert(&self) -> bool {
        matches!(self, Mode::Insert(..) | Mode::Search(..))
    }
}

pub(super) trait EditorMode {
    fn process_event(&mut self, event: Event, _line: &Line, cmds: &mut Vec<Command>);
}

fn parse_vim_text_object(
    sel: char,
    obj: char,
) -> Option<(text_object::Selector, text_object::TextObject)> {
    use text_object::{Selector, TextObject};

    let sel = match sel {
        'i' => Selector::Inside,
        'a' => Selector::An,
        _ => return None,
    };

    let obj = match obj {
        'w' => TextObject::Word { wide: false },
        'W' => TextObject::Word { wide: true },
        '(' | ')' => TextObject::Pair {
            begin: '(',
            end: ')',
        },
        '[' | ']' => TextObject::Pair {
            begin: '[',
            end: ']',
        },
        '<' | '>' => TextObject::Pair {
            begin: '<',
            end: '>',
        },
        '{' | '}' => TextObject::Pair {
            begin: '{',
            end: '}',
        },
        '\'' => TextObject::Pair {
            begin: '\'',
            end: '\'',
        },
        '"' => TextObject::Pair {
            begin: '"',
            end: '"',
        },
        '`' => TextObject::Pair {
            begin: '`',
            end: '`',
        },
        _ => return None,
    };

    Some((sel, obj))
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct NormalMode {
    combo: Vec<char>,
    last_find: Option<(char, char)>,
}

impl NormalMode {
    fn process_text_object(&mut self, event: Event, line: &Line, cmds: &mut Vec<Command>) {
        if self.combo.len() < 3 {
            match event {
                Event::Char(ch) => {
                    self.combo.push(ch);
                    if self.combo.len() < 3 {
                        return;
                    }
                }
                _ => {
                    self.combo.clear();
                    return;
                }
            }
        }

        if let Some((sel, obj)) = parse_vim_text_object(self.combo[1], self.combo[2]) {
            let (from, to) = text_object::find_range(line, sel, obj);
            let selected: String = line.iter(from..to).map(|(c, _)| c).collect();

            match self.combo[0] {
                'd' => {
                    cmds.push(Command::MakeCheckPoint);
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: selected,
                    });
                    cmds.push(Command::DeleteRange { from, to });
                }
                'c' => {
                    cmds.push(Command::MakeCheckPoint);
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: selected,
                    });
                    cmds.push(Command::DeleteRange { from, to });
                    cmds.push(Command::ChangeModeToInsert);
                }
                'y' => {
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: selected,
                    });
                    cmds.push(Command::CursorExact(from));
                }
                _ => unreachable!(),
            }
        }

        self.combo.clear();
    }
}

impl EditorMode for NormalMode {
    fn process_event(&mut self, event: Event, line: &Line, cmds: &mut Vec<Command>) {
        match self.combo.first() {
            None => match event {
                Event::Char('i') => {
                    cmds.push(Command::MakeCheckPoint);
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
                    cmds.push(Command::CursorExact(0));
                }

                Event::Char('A') => {
                    cmds.push(Command::MakeCheckPoint);
                    cmds.push(Command::ChangeModeToInsert);
                    cmds.push(Command::CursorEnd);
                }
                Event::Char('I') => {
                    cmds.push(Command::MakeCheckPoint);
                    cmds.push(Command::ChangeModeToInsert);
                    cmds.push(Command::CursorBegin);
                }

                Event::Char('a') => {
                    cmds.push(Command::MakeCheckPoint);
                    cmds.push(Command::ChangeModeToInsert);
                    cmds.push(Command::CursorNextChar);
                }
                Event::Char('s') => {
                    cmds.push(Command::MakeCheckPoint);

                    if let Some(ch) = line.char_at(line.cursor()) {
                        cmds.push(Command::RegisterStore {
                            reg: '"',
                            text: ch.to_string(),
                        });
                    }

                    cmds.push(Command::ChangeModeToInsert);
                    cmds.push(Command::DeleteNextChar);
                }
                Event::Char('x') => {
                    cmds.push(Command::MakeCheckPoint);

                    if let Some(ch) = line.char_at(line.cursor()) {
                        cmds.push(Command::RegisterStore {
                            reg: '"',
                            text: ch.to_string(),
                        });
                    }

                    cmds.push(Command::DeleteNextChar);
                }

                Event::Char('d') => {
                    self.combo.push('d');
                }
                Event::Char('c') => {
                    self.combo.push('c');
                }

                Event::Char('D') => {
                    cmds.push(Command::MakeCheckPoint);

                    let from = line.cursor();
                    let to = line.len();
                    let cursor_to_end: String = line.iter(from..to).map(|(c, _)| c).collect();
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: cursor_to_end,
                    });
                    cmds.push(Command::DeleteRange { from, to });
                }
                Event::Char('C') => {
                    cmds.push(Command::MakeCheckPoint);

                    let from = line.cursor();
                    let to = line.len();
                    let cursor_to_end: String = line.iter(from..to).map(|(c, _)| c).collect();
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: cursor_to_end,
                    });

                    cmds.push(Command::ChangeModeToInsert);
                    cmds.push(Command::DeleteRange { from, to });
                }
                Event::Char('S') => {
                    cmds.push(Command::MakeCheckPoint);
                    cmds.push(Command::DeleteLine);
                    cmds.push(Command::ChangeModeToInsert);
                }

                Event::Char('y') => {
                    self.combo.push('y');
                }
                Event::Char('Y') => {
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: line.to_string(),
                    });
                }

                Event::Char('P') => {
                    cmds.push(Command::MakeCheckPoint);
                    cmds.push(Command::RegisterPastePrev { reg: '"' });
                }
                Event::Char('p') => {
                    cmds.push(Command::MakeCheckPoint);
                    cmds.push(Command::RegisterPasteNext { reg: '"' });
                }

                Event::Char('u') => {
                    cmds.push(Command::Undo);
                }
                Event::Ctrl('r') => {
                    cmds.push(Command::Redo);
                }

                Event::Ctrl('o') => cmds.push(Command::CdUndo),
                Event::KeyTab => cmds.push(Command::CdRedo),
                Event::Ctrl('p') => cmds.push(Command::CdToParent),

                _ => {}
            },

            Some('d') => {
                if self.combo.len() == 1 && event == Event::Char('d') {
                    cmds.push(Command::MakeCheckPoint);
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: line.to_string(),
                    });
                    cmds.push(Command::DeleteLine);
                    self.combo.clear();
                } else {
                    self.process_text_object(event, line, cmds);
                }
            }

            Some('c') => {
                if self.combo.len() == 1 && event == Event::Char('c') {
                    cmds.push(Command::MakeCheckPoint);
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: line.to_string(),
                    });
                    cmds.push(Command::DeleteLine);
                    cmds.push(Command::ChangeModeToInsert);
                    self.combo.clear();
                } else {
                    self.process_text_object(event, line, cmds);
                }
            }

            Some('y') => {
                if self.combo.len() == 1 && event == Event::Char('y') {
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: line.to_string(),
                    });
                    self.combo.clear();
                } else {
                    self.process_text_object(event, line, cmds);
                }
            }

            Some('f') => {
                if let Event::Char(ch) = event {
                    self.last_find = Some(('f', ch));
                    cmds.push(Command::CursorNextCharMatch(ch));
                } else {
                    self.last_find = None;
                }
                self.combo.clear();
            }
            Some('F') => {
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

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SearchMode {
    query: Line,
}

impl SearchMode {
    pub fn new() -> Self {
        Self { query: Line::new() }
    }

    pub fn query(&self) -> String {
        self.query.to_string()
    }
}

impl EditorMode for SearchMode {
    fn process_event(&mut self, event: Event, _line: &Line, cmds: &mut Vec<Command>) {
        match event {
            Event::KeyEscape | Event::KeyTab | Event::Ctrl('u') | Event::Ctrl('d') => {
                cmds.push(Command::ChangeModeToInsert);
            }

            Event::KeyReturn => {
                cmds.push(Command::ChangeModeToInsert);
                cmds.push(Command::Commit);
            }
            Event::KeyLeft => {
                cmds.push(Command::ChangeModeToInsert);
                cmds.push(Command::CursorPrevChar);
            }
            Event::KeyRight => {
                cmds.push(Command::ChangeModeToInsert);
                cmds.push(Command::CursorNextChar);
            }
            Event::KeyUp => {
                cmds.push(Command::ChangeModeToInsert);
                cmds.push(Command::HistoryPrev);
            }
            Event::KeyDown => {
                cmds.push(Command::ChangeModeToInsert);
                cmds.push(Command::HistoryNext);
            }

            Event::Char(ch) => {
                self.query.insert(ch);
                cmds.push(Command::HistorySearch {
                    query: self.query.to_string(),
                    reset: true,
                });
            }
            Event::KeyBackspace => {
                self.query.delete_prev();
                cmds.push(Command::HistorySearch {
                    query: self.query.to_string(),
                    reset: true,
                });
            }
            Event::Ctrl('w') => {
                self.query.delete_word();
                cmds.push(Command::HistorySearch {
                    query: self.query.to_string(),
                    reset: true,
                });
            }

            Event::Ctrl('r') => {
                cmds.push(Command::HistorySearch {
                    query: self.query.to_string(),
                    reset: false,
                });
            }

            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct InsertMode;

impl EditorMode for InsertMode {
    fn process_event(&mut self, event: Event, _line: &Line, cmds: &mut Vec<Command>) {
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
            Event::Ctrl('u') => cmds.push(Command::DeleteLine),

            Event::KeyTab => cmds.push(Command::TryCompleteFilename),
            Event::Ctrl('d') => cmds.push(Command::DisplayCompletionCandidate),

            Event::Ctrl('p') => cmds.push(Command::CdToParent),
            Event::Ctrl('o') => cmds.push(Command::CdUndo),

            Event::Ctrl('r') => {
                cmds.push(Command::ChangeModeToSearch);
            }

            Event::Ctrl('n') => cmds.push(Command::DuplicateWord),

            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct VisualMode {
    origin: isize,
    combo: Vec<char>,
}

impl VisualMode {
    pub fn new_char(origin: usize) -> Self {
        Self {
            origin: origin as isize,
            combo: Vec::new(),
        }
    }

    pub fn new_line() -> Self {
        Self {
            origin: isize::MIN,
            combo: Vec::new(),
        }
    }

    pub fn origin(&self) -> Option<usize> {
        if self.is_line_mode() {
            None
        } else {
            Some(self.origin as usize)
        }
    }

    fn is_line_mode(&self) -> bool {
        self.origin == isize::MIN
    }

    fn process_text_object(&mut self, event: Event, line: &Line, cmds: &mut Vec<Command>) {
        if self.combo.len() < 2 {
            match event {
                Event::Char(ch) => {
                    self.combo.push(ch);
                    if self.combo.len() < 2 {
                        return;
                    }
                }
                _ => {
                    self.combo.clear();
                    return;
                }
            }
        }

        if let Some((sel, obj)) = parse_vim_text_object(self.combo[0], self.combo[1]) {
            let (from, to) = text_object::find_range(line, sel, obj);
            if to > from {
                self.origin = from as isize;
                cmds.push(Command::CursorExact(to - 1));
            }
        }

        self.combo.clear();
    }
}

impl EditorMode for VisualMode {
    fn process_event(&mut self, event: Event, line: &Line, cmds: &mut Vec<Command>) {
        match self.combo.first() {
            None => {
                match event {
                    Event::KeyEscape | Event::Char('v') => {
                        cmds.push(Command::ChangeModeToNormal);
                    }

                    Event::Char(sel @ ('i' | 'a')) => {
                        self.combo.push(sel);
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

                    Event::Char('o') => {
                        if !self.is_line_mode() {
                            cmds.push(Command::CursorExact(self.origin as usize));
                            self.origin = line.cursor() as isize;
                        }
                    }

                    Event::Char('$') => {
                        cmds.push(Command::CursorEnd);
                    }
                    Event::Char('^') => {
                        cmds.push(Command::CursorBegin);
                    }
                    Event::Char('0') => {
                        cmds.push(Command::CursorExact(0));
                    }

                    Event::Char('D') => {
                        cmds.push(Command::MakeCheckPoint);
                        cmds.push(Command::RegisterStore {
                            reg: '"',
                            text: line.to_string(),
                        });
                        cmds.push(Command::DeleteLine);
                        cmds.push(Command::ChangeModeToNormal);
                    }
                    Event::Char('C') | Event::Char('S') => {
                        cmds.push(Command::MakeCheckPoint);
                        cmds.push(Command::RegisterStore {
                            reg: '"',
                            text: line.to_string(),
                        });
                        cmds.push(Command::ChangeModeToInsert);
                        cmds.push(Command::DeleteLine);
                    }
                    Event::Char('Y') => {
                        cmds.push(Command::RegisterStore {
                            reg: '"',
                            text: line.to_string(),
                        });
                        cmds.push(Command::ChangeModeToNormal);
                    }

                    Event::Char('d') | Event::Char('x') => {
                        cmds.push(Command::MakeCheckPoint);

                        if self.is_line_mode() {
                            cmds.push(Command::RegisterStore {
                                reg: '"',
                                text: line.to_string(),
                            });

                            cmds.push(Command::DeleteLine);
                        } else {
                            let mut from = self.origin as usize;
                            let mut to = line.cursor();
                            if from > to {
                                std::mem::swap(&mut from, &mut to);
                            }
                            to += 1; // make it half-opened

                            let part: String = line.iter(from..to).map(|(ch, _)| ch).collect();
                            cmds.push(Command::RegisterStore {
                                reg: '"',
                                text: part,
                            });

                            cmds.push(Command::DeleteRange { from, to });
                        }
                        cmds.push(Command::ChangeModeToNormal);
                    }
                    Event::Char('c') | Event::Char('s') => {
                        cmds.push(Command::MakeCheckPoint);

                        cmds.push(Command::ChangeModeToInsert);
                        if self.is_line_mode() {
                            cmds.push(Command::RegisterStore {
                                reg: '"',
                                text: line.to_string(),
                            });

                            cmds.push(Command::DeleteLine);
                        } else {
                            let mut from = self.origin as usize;
                            let mut to = line.cursor();
                            if from > to {
                                std::mem::swap(&mut from, &mut to);
                            }
                            to += 1; // make it half-opened

                            let part: String = line.iter(from..to).map(|(ch, _)| ch).collect();
                            cmds.push(Command::RegisterStore {
                                reg: '"',
                                text: part,
                            });

                            cmds.push(Command::DeleteRange { from, to });
                        }
                    }
                    Event::Char('y') => {
                        if self.is_line_mode() {
                            cmds.push(Command::RegisterStore {
                                reg: '"',
                                text: line.to_string(),
                            });

                            cmds.push(Command::DeleteLine);
                        } else {
                            let mut from = self.origin as usize;
                            let mut to = line.cursor();
                            if from > to {
                                std::mem::swap(&mut from, &mut to);
                            }
                            to += 1; // make it half-opened

                            let part: String = line.iter(from..to).map(|(ch, _)| ch).collect();
                            cmds.push(Command::RegisterStore {
                                reg: '"',
                                text: part,
                            });
                        }
                        cmds.push(Command::ChangeModeToNormal);
                    }

                    _ => {}
                }
            }
            Some(_) => {
                self.process_text_object(event, line, cmds);
            }
        }
    }
}
