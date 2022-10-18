use std::path::{Path, PathBuf};

pub(super) struct FileCompletion {
    base_dir: Option<PathBuf>,
}

impl FileCompletion {
    pub fn new_cwd() -> Self {
        Self {
            base_dir: std::env::current_dir().ok(),
        }
    }

    pub fn candidates(&self, partial: &str) -> Vec<String> {
        self.find(partial).unwrap_or_default()
    }

    fn find(&self, partial: &str) -> Option<Vec<String>> {
        let mut path = self.base_dir.clone()?;
        let partial = Path::new(partial);
        if partial.is_relative() {
            path.extend(partial);
        } else {
            path = partial.to_owned();
        }

        let dir_name = if path.is_dir() {
            path.as_path()
        } else {
            path.parent()?
        };

        let file_name = if path.is_dir() {
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
            candidates.last_mut().unwrap().push('/');
        }

        Some(candidates)
    }
}
