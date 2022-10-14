#[derive(Clone)]
pub struct Line {
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
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            cursor: 0,
        }
    }

    pub fn iter(
        &self,
        range: impl std::slice::SliceIndex<[(char, usize)], Output = [(char, usize)]>,
    ) -> impl Iterator<Item = (char, usize)> + '_ {
        self.buf.get(range).unwrap().iter().copied()
    }

    pub fn char_at(&self, at: usize) -> Option<char> {
        self.buf.get(at).map(|(ch, _)| ch).copied()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert(&mut self, ch: char) {
        use unicode_width::UnicodeWidthChar as _;
        let width = ch.width().unwrap_or(1);

        self.buf.insert(self.cursor, (ch, width));
        self.cursor += 1;
    }

    pub fn delete_prev(&mut self) {
        if self.cursor > 0 {
            self.buf.remove(self.cursor - 1);
            self.cursor -= 1;
        }
    }

    pub fn delete_next(&mut self) {
        if self.cursor < self.buf.len() {
            self.buf.remove(self.cursor);
        }
    }

    pub fn delete_word(&mut self) {
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

    pub fn delete_line(&mut self) {
        self.cursor = 0;
        self.buf.clear();
    }

    // delete characters in [from, to)
    pub fn delete_range(&mut self, from: usize, to: usize) {
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

    pub fn cursor_prev_char(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn cursor_next_char(&mut self) {
        if self.cursor < self.buf.len() {
            self.cursor += 1;
        }
    }

    pub fn cursor_prev_char_match(&mut self, target: char) {
        let mut i = self.cursor as isize - 1;
        while i > 0 {
            if self.buf[i as usize].0 == target {
                self.cursor = i as usize;
                break;
            }
            i -= 1;
        }
    }

    pub fn cursor_next_char_match(&mut self, target: char) {
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

    pub fn cursor_prev_word_head(&mut self, _wide: bool) {
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

    pub fn cursor_next_word_head(&mut self, _wide: bool) {
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

    pub fn cursor_next_word_end(&mut self, _wide: bool) {
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

    pub fn cursor_end_of_line(&mut self) {
        self.cursor = self.buf.len();
    }

    pub fn cursor_begin_of_line(&mut self) {
        let len = self.buf.len();

        self.cursor = 0;
        while self.cursor <= len {
            if !self.buf[self.cursor].0.is_whitespace() {
                break;
            }
            self.cursor += 1;
        }
    }

    pub fn cursor_exact(&mut self, pos: usize) {
        self.cursor = pos;
    }

    pub fn normal_mode_fix_cursor(&mut self) {
        if self.cursor >= self.buf.len() {
            self.cursor = (self.buf.len() as isize - 1).max(0) as usize;
        }
    }
}
