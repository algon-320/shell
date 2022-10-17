use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Mode {
    Insert(InsertMode),
    Normal(NormalMode),
    Visual(VisualMode),
}

impl Mode {
    pub fn is_insert(&self) -> bool {
        matches!(self, Mode::Insert(..))
    }
}

pub(super) trait EditorMode {
    fn process_event(&mut self, event: Event, _line: &Line, cmds: &mut Vec<Command>);
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct NormalMode {
    combo: String,
    last_find: Option<(char, char)>,
}

impl EditorMode for NormalMode {
    fn process_event(&mut self, event: Event, line: &Line, cmds: &mut Vec<Command>) {
        match self.combo.as_str() {
            "" => match event {
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
                    cmds.push(Command::CursorExact(0));
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
                    let ch = line.char_at(line.cursor()).unwrap();
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: ch.to_string(),
                    });

                    cmds.push(Command::ChangeModeToInsert);
                    cmds.push(Command::DeleteNextChar);
                    cmds.push(Command::MakeCheckPoint);
                }
                Event::Char('x') => {
                    let ch = line.char_at(line.cursor()).unwrap();
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: ch.to_string(),
                    });

                    cmds.push(Command::DeleteNextChar);
                    cmds.push(Command::MakeCheckPoint);
                }

                Event::Char('d') => {
                    self.combo.push('d');
                }
                Event::Char('c') => {
                    self.combo.push('c');
                }

                Event::Char('D') => {
                    let from = line.cursor();
                    let to = line.len();
                    let cursor_to_end: String = line.iter(from..to).map(|(c, _)| c).collect();
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: cursor_to_end,
                    });
                    cmds.push(Command::DeleteRange { from, to });
                    cmds.push(Command::MakeCheckPoint);
                }
                Event::Char('C') => {
                    let from = line.cursor();
                    let to = line.len();
                    let cursor_to_end: String = line.iter(from..to).map(|(c, _)| c).collect();
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: cursor_to_end,
                    });

                    cmds.push(Command::ChangeModeToInsert);
                    cmds.push(Command::DeleteRange { from, to });
                    cmds.push(Command::MakeCheckPoint);
                }
                Event::Char('S') => {
                    cmds.push(Command::DeleteLine);
                    cmds.push(Command::ChangeModeToInsert);
                    cmds.push(Command::MakeCheckPoint);
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
                    cmds.push(Command::RegisterPastePrev { reg: '"' });
                    cmds.push(Command::MakeCheckPoint);
                }
                Event::Char('p') => {
                    cmds.push(Command::RegisterPasteNext { reg: '"' });
                    cmds.push(Command::MakeCheckPoint);
                }

                Event::Char('u') => {
                    cmds.push(Command::Undo);
                }
                Event::Ctrl('r') => {
                    cmds.push(Command::Redo);
                }

                _ => {}
            },

            "d" => {
                if let Event::Char('d') = event {
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: line.to_string(),
                    });

                    cmds.push(Command::DeleteLine);
                    cmds.push(Command::MakeCheckPoint);
                }
                self.combo.clear();
            }

            "c" => {
                if let Event::Char('c') = event {
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: line.to_string(),
                    });

                    cmds.push(Command::DeleteLine);
                    cmds.push(Command::ChangeModeToInsert);
                    cmds.push(Command::MakeCheckPoint);
                }
                self.combo.clear();
            }

            "y" => {
                if let Event::Char('y') = event {
                    cmds.push(Command::RegisterStore {
                        reg: '"',
                        text: line.to_string(),
                    });
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
pub(super) struct InsertMode;

impl EditorMode for InsertMode {
    fn process_event(&mut self, event: Event, _line: &Line, cmds: &mut Vec<Command>) {
        match event {
            Event::KeyEscape => {
                cmds.push(Command::CursorPrevChar);
                cmds.push(Command::ChangeModeToNormal);
                cmds.push(Command::MakeCheckPoint);
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

            Event::KeyTab => {
                // TODO: completion
            }

            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct VisualMode {
    origin: isize,
}

impl VisualMode {
    pub fn new_char(origin: usize) -> Self {
        Self {
            origin: origin as isize,
        }
    }

    pub fn new_line() -> Self {
        Self { origin: isize::MIN }
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
}

impl EditorMode for VisualMode {
    fn process_event(&mut self, event: Event, line: &Line, cmds: &mut Vec<Command>) {
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
                cmds.push(Command::RegisterStore {
                    reg: '"',
                    text: line.to_string(),
                });

                cmds.push(Command::DeleteLine);
                cmds.push(Command::ChangeModeToNormal);
                cmds.push(Command::MakeCheckPoint);
            }
            Event::Char('C') | Event::Char('S') => {
                cmds.push(Command::RegisterStore {
                    reg: '"',
                    text: line.to_string(),
                });

                cmds.push(Command::ChangeModeToInsert);
                cmds.push(Command::DeleteLine);
                cmds.push(Command::MakeCheckPoint);
            }
            Event::Char('Y') => {
                cmds.push(Command::RegisterStore {
                    reg: '"',
                    text: line.to_string(),
                });
                cmds.push(Command::ChangeModeToNormal);
            }

            Event::Char('d') | Event::Char('x') => {
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
                cmds.push(Command::MakeCheckPoint);
            }
            Event::Char('c') | Event::Char('s') => {
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
                cmds.push(Command::MakeCheckPoint);
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
}
