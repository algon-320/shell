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
        self.commands = StaticWordCompletion::new(new_commands);
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
        let mut path = std::env::current_dir().ok()?;

        if partial.starts_with('~') {
            use std::ffi::OsString;
            use std::os::unix::ffi::OsStringExt as _;

            let expanded = expand_tilde(partial.as_bytes());
            path = PathBuf::from(OsString::from_vec(expanded));
        } else {
            let partial_path = Path::new(partial);
            if partial_path.is_absolute() {
                path = partial_path.to_owned();
            } else {
                path.extend(partial_path);
            }
        }

        let dir_name = if partial.ends_with(std::path::MAIN_SEPARATOR) {
            path.as_path()
        } else {
            path.parent()?
        };

        let file_name = if partial.ends_with(std::path::MAIN_SEPARATOR) {
            ""
        } else {
            path.file_name()?.to_str()?
        };

        let entries = std::fs::read_dir(dir_name).ok()?;

        let mut candidates = Vec::new();
        let mut is_dir = Vec::new();

        for ent in entries.filter_map(|ent| ent.ok()) {
            let ent_name = ent.file_name();
            if let Some(tail) = ent_name.to_str().and_then(|s| s.strip_prefix(file_name)) {
                let tail = tail.to_owned();
                candidates.push(tail);
                is_dir.push(ent.metadata().map(|meta| meta.is_dir()).unwrap());
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
