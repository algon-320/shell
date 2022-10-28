use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub trait Complete {
    fn candidates(&self, words: &[&str]) -> Vec<String>;
}

pub struct CommandCompletion {
    commands: StaticWordCompletion,
    rules: HashMap<String, Box<dyn Complete>>,
    fallback: Box<dyn Complete>,
}

impl CommandCompletion {
    pub fn new(commands: Vec<String>, fallback: Box<dyn Complete>) -> Self {
        Self {
            commands: StaticWordCompletion::new(commands),
            rules: HashMap::new(),
            fallback,
        }
    }

    pub fn update_commands(&mut self, new_commands: Vec<String>) {
        self.commands = StaticWordCompletion::new(new_commands.clone());

        // FIXME
        self.rules.insert(
            "sudo".to_owned(),
            Box::new(StaticWordCompletion::new(new_commands)),
        );
    }

    #[allow(unused)]
    pub fn add_completion(&mut self, cmd: String, completion: Box<dyn Complete>) {
        self.rules.insert(cmd, completion);
    }
}

impl Complete for CommandCompletion {
    fn candidates(&self, words: &[&str]) -> Vec<String> {
        if words.len() <= 1 {
            self.commands.candidates(words)
        } else if let Some(comp) = self.rules.get(words[0]) {
            comp.candidates(words)
        } else {
            self.fallback.candidates(words)
        }
    }
}

pub struct StaticWordCompletion {
    items: Vec<String>,
}

impl StaticWordCompletion {
    pub fn new(items: Vec<String>) -> Self {
        Self { items }
    }
}

impl Complete for StaticWordCompletion {
    fn candidates(&self, words: &[&str]) -> Vec<String> {
        if let Some(word) = words.last() {
            self.items
                .iter()
                .filter_map(|item| item.strip_prefix(word))
                .map(str::to_owned)
                .collect()
        } else {
            Vec::new()
        }
    }
}

use crate::core::expand_tilde;

pub struct FileCompletion(());

impl FileCompletion {
    pub fn new() -> Self {
        Self(())
    }

    fn find(&self, partial: &str) -> Option<Vec<String>> {
        let mut path = if partial.starts_with('~') {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt as _;

            let expanded = expand_tilde(partial.as_bytes());
            PathBuf::from(OsString::from_vec(expanded))
        } else {
            Path::new(partial).to_owned()
        };

        if path.is_relative() {
            let mut tmp = std::env::current_dir().ok()?;
            tmp.extend(path.as_path());
            path = tmp;
        }

        let (dir, pat);
        if partial.ends_with(std::path::MAIN_SEPARATOR) || partial.is_empty() {
            dir = path.as_path();
            pat = "";
        } else {
            dir = path.parent()?;
            pat = path.file_name()?.to_str()?;
        }

        let mut candidates = Vec::new();
        let mut is_dir = Vec::new();

        let entries = std::fs::read_dir(dir).ok()?;
        for ent in entries.filter_map(|ent| ent.ok()) {
            if let Some(stripped) = ent.file_name().to_str().and_then(|s| s.strip_prefix(pat)) {
                let cand = Self::escape_special_characters(stripped);
                candidates.push(cand);

                let ent_is_dir = ent.metadata().map(|m| m.is_dir()).unwrap_or(false);
                is_dir.push(ent_is_dir);
            }
        }

        // append a slash if there is a single candidate
        if candidates.len() == 1 && is_dir[0] {
            candidates
                .last_mut()
                .unwrap()
                .push(std::path::MAIN_SEPARATOR);
        }

        Some(candidates)
    }

    fn escape_special_characters(candidate: &str) -> String {
        // example:
        //   "foo bar" --> "foo\ bar"
        //   "foo@bar" --> "foo\@bar"

        let mut buf = String::new();
        for ch in candidate.chars() {
            if let '\\' | ' ' | '\t' | '\n' | '@' | ';' | '&' | '|' | '$' | '(' | ')' | '[' | ']'
            | '\'' | '\"' | '=' | '?' | '{' | '}' = ch
            {
                buf.push('\\');
            }
            buf.push(ch);
        }
        buf
    }
}

impl Complete for FileCompletion {
    fn candidates(&self, words: &[&str]) -> Vec<String> {
        if let Some(word) = words.last() {
            self.find(word).unwrap_or_default()
        } else {
            Vec::new()
        }
    }
}
