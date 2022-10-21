use crate::line_editor::{CharClass, Line};

pub enum Selector {
    An,
    Inside,
}

pub enum TextObject {
    Word { wide: bool },
    Pair { begin: char, end: char },
}

pub fn find_range(line: &Line, selector: Selector, object: TextObject) -> (usize, usize) {
    match (selector, object) {
        (Selector::Inside, TextObject::Word { wide }) => {
            let cursor = line.cursor();

            let word_class;
            if let Some(ch) = line.char_at(cursor) {
                word_class = CharClass::from(ch);
            } else {
                return (0, 0);
            }

            let mut i = cursor;
            while i > 0 {
                let prev_class = CharClass::from(line.char_at(i - 1).unwrap());
                if !CharClass::is_same(wide, prev_class, word_class) {
                    break;
                }
                i -= 1;
            }
            let from = i;

            let mut i = cursor;
            while i < line.len() {
                let class = CharClass::from(line.char_at(i).unwrap());
                if !CharClass::is_same(wide, class, word_class) {
                    break;
                }
                i += 1;
            }
            let to = i;

            (from, to)
        }

        (Selector::An, TextObject::Word { wide }) => {
            let cursor = line.cursor();

            let word_class;
            if let Some(ch) = line.char_at(cursor) {
                word_class = CharClass::from(ch);
            } else {
                return (0, 0);
            }

            let mut i = cursor;
            while i > 0 {
                let prev_class = CharClass::from(line.char_at(i - 1).unwrap());
                if !CharClass::is_same(wide, prev_class, word_class) {
                    break;
                }
                i -= 1;
            }
            while i > 0 {
                let prev_class = CharClass::from(line.char_at(i - 1).unwrap());
                if !prev_class.is_whitespace() {
                    break;
                }
                i -= 1;
            }
            let from = i;

            let mut i = cursor;
            while i < line.len() {
                let class = CharClass::from(line.char_at(i).unwrap());
                if !CharClass::is_same(wide, class, word_class) {
                    break;
                }
                i += 1;
            }
            while i < line.len() {
                let class = CharClass::from(line.char_at(i).unwrap());
                if !class.is_whitespace() {
                    break;
                }
                i += 1;
            }
            let to = i;

            (from, to)
        }

        (Selector::Inside, TextObject::Pair { begin, end }) => {
            let cursor = line.cursor();

            let mut i = cursor;
            while i > 0 {
                if line.char_at(i - 1).unwrap() == begin {
                    break;
                }
                i -= 1;
            }
            let from = i;

            let mut i = cursor;
            while i < line.len() {
                if line.char_at(i).unwrap() == end {
                    break;
                }
                i += 1;
            }
            let to = i;

            (from, to)
        }

        (Selector::An, TextObject::Pair { begin, end }) => {
            let cursor = line.cursor();

            let mut i = cursor;
            while i > 0 {
                if line.char_at(i).unwrap() == begin {
                    break;
                }
                i -= 1;
            }
            let from = i;

            let mut i = cursor;
            while i < line.len() {
                if line.char_at(i).unwrap() == end {
                    i += 1;
                    break;
                }
                i += 1;
            }
            let to = i;

            (from, to)
        }
    }
}
