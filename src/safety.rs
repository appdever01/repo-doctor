use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

pub const FILE_LIMIT: usize = 1024 * 1024;
pub const TOTAL_LIMIT: usize = 16 * FILE_LIMIT;
pub const OUTPUT_LIMIT: usize = 16 * 1024;
pub const RUN_LIMIT: Duration = Duration::from_secs(15);

pub fn safe_text(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(512)
        .collect()
}

pub fn variable_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .enumerate()
            .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

pub fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

pub fn relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.chars().any(char::is_control)
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

pub struct FileReader {
    pub root: PathBuf,
    pub deadline: Instant,
    total: usize,
}

impl FileReader {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            deadline: Instant::now() + RUN_LIMIT,
            total: 0,
        }
    }

    pub fn boundary(&self, name: &str) -> Result<(), &'static str> {
        if !relative_path(name) {
            return Err("The file path must remain inside the selected project.");
        }
        let selected = self.root.join(name);
        let mut candidate = selected.as_path();
        loop {
            match candidate.canonicalize() {
                Ok(real) if real.starts_with(&self.root) => return Ok(()),
                Ok(_) => return Err("A symlink leads outside the selected project."),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if std::fs::symlink_metadata(candidate).is_ok() {
                        return Err("The file path contains an unresolved symlink.");
                    }
                    candidate = candidate
                        .parent()
                        .ok_or("The file could not be resolved.")?;
                }
                Err(_) => return Err("The file path could not be inspected."),
            }
        }
    }

    pub fn present(&self, name: &str) -> Result<bool, &'static str> {
        let selected = self.root.join(name);
        match selected.canonicalize() {
            Ok(path) if !path.starts_with(&self.root) => {
                Err("A symlink leads outside the selected project.")
            }
            Ok(path) if !path.is_file() => Err("The input must be a regular file."),
            Ok(_) => Ok(true),
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    && std::fs::symlink_metadata(selected).is_err() =>
            {
                Ok(false)
            }
            Err(_) => Err("The input could not be inspected."),
        }
    }

    pub fn read(&mut self, name: &str) -> Result<Option<String>, &'static str> {
        if Instant::now() >= self.deadline {
            return Err("The whole-run deadline was exceeded.");
        }
        if !relative_path(name) {
            return Err("The file path must remain inside the selected project.");
        }
        let selected = self.root.join(name);
        let path = match selected.canonicalize() {
            Ok(path) => path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut parent = selected.as_path();
                while let Some(p) = parent.parent() {
                    if let Ok(real) = p.canonicalize() {
                        if !real.starts_with(&self.root) {
                            return Err("A symlink leads outside the selected project.");
                        }
                        break;
                    }
                    parent = p;
                }
                if std::fs::symlink_metadata(&selected).is_ok() {
                    return Err("The file is an unresolved symlink.");
                }
                return Ok(None);
            }
            Err(_) => return Err("The file could not be accessed."),
        };
        if !path.starts_with(&self.root) {
            return Err("A symlink leads outside the selected project.");
        }
        let metadata = path
            .metadata()
            .map_err(|_| "The file could not be inspected.")?;
        if !metadata.is_file() {
            return Err("The input must be a regular file.");
        }
        if metadata.len() > FILE_LIMIT as u64 {
            return Err("The file exceeds the 1 MiB input limit.");
        }
        let file = File::open(path).map_err(|_| "The file could not be read.")?;
        let mut bytes = Vec::new();
        file.take((FILE_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "The file could not be read.")?;
        if bytes.len() > FILE_LIMIT {
            return Err("The file exceeds the 1 MiB input limit.");
        }
        self.total += bytes.len();
        if self.total > TOTAL_LIMIT {
            return Err("Inputs exceed the combined 16 MiB limit.");
        }
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "The file is not valid UTF-8.")
    }
}
