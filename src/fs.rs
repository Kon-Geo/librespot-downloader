use std::{collections::HashMap, fs, path::Path};
use crate::metadata::TrackFileDescriptor;

pub type FileOccurences = HashMap<String, Vec<TrackFileDescriptor>>;

pub fn collect_file_occurences<F>(path: &Path, filter: &F) -> FileOccurences
where
    F: Fn(&str) -> Option<String>,
{
    let mut occurrences = FileOccurences::new();
    let Ok(entries) = fs::read_dir(path) else {
        return occurrences;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            for (file, folders) in collect_file_occurences(&path, filter) {
                occurrences.entry(file).or_default().extend(folders);
            }
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue; };
        let Some(base) = filter(stem) else { continue; };
        let Some(extension) = path.extension().and_then(|s| s.to_str()) else { continue; };
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue; };
        let file = TrackFileDescriptor {
            base: base.clone(),
            stem: stem.to_string(),
            extension: extension.to_string(),
            name: name.to_string(),
            path: path.clone(),
        };
        occurrences.entry(base).or_default().push(file);
    }
    occurrences
}

/// Deletes leftover `.part` files from interrupted downloads under `path`.
pub fn remove_stale_parts(path: &Path) {
    let Ok(entries) = fs::read_dir(path) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_stale_parts(&path);
        } else if path.extension().and_then(|e| e.to_str()) == Some("part") {
            let _ = fs::remove_file(&path);
        }
    }
}

/// Splits "<base> (<22-char base62 id>)" into `<base>`. Returns None for files
/// that don't follow this downloader's naming scheme.
pub fn strip_id_suffix(stem: &str) -> Option<String> {
    split_id_suffix(stem).map(|(base, _)| base)
}

/// Splits "<base> (<22-char base62 id>)" into `(<base>, <id>)`.
pub fn split_id_suffix(stem: &str) -> Option<(String, String)> {
    let inner = stem.strip_suffix(')')?;
    let open = inner.rfind(" (")?;
    let id = &inner[open + 2..];
    if id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some((inner[..open].to_string(), id.to_string()))
    } else {
        None
    }
}

/// Finds the track id of any downloaded file under `dir` (ignores `.part` files).
pub fn first_track_id(dir: &Path) -> Option<String> {
    let mut subdirs = Vec::new();
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if path.extension().and_then(|e| e.to_str()) != Some("part") {
            if let Some((_, id)) = path.file_stem().and_then(|s| s.to_str()).and_then(split_id_suffix) {
                return Some(id);
            }
        }
    }
    subdirs.iter().find_map(|d| first_track_id(d))
}

pub struct FileDB {
    pub files: FileOccurences,
    pub tracked_artists: Vec<String>,
}

impl FileDB {
    pub fn new() -> Self {
        Self { files: FileOccurences::new(), tracked_artists: Vec::new() }
    }

    pub fn remove_occurrence(&mut self, name: &str, path: &Path) {
        if let Some(paths) = self.files.get_mut(name) {
            paths.retain(|file| file.path != path);

            if paths.is_empty() {
                self.files.remove(name);
            }
        }
    }

    pub fn track_artist(&mut self, b62id: &String, folder: &Path) {
        if self.tracked_artists.contains(&b62id) {
            return;
        }
        remove_stale_parts(folder);
        let occurrences = collect_file_occurences(folder, &strip_id_suffix);
        for (base, files) in occurrences {
            self.files.entry(base).or_default().extend(files);
        }
        self.tracked_artists.push(b62id.clone());
    }

    pub fn track_track(&mut self, file: TrackFileDescriptor) {
        self.files
            .entry(file.base.clone())
            .or_default()
            .push(file);
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn collects_files_recursively_by_base() {
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        let root = std::env::temp_dir().join(format!("lsd-fs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Album")).unwrap();
        fs::create_dir_all(root.join("Singles")).unwrap();
        fs::write(root.join("Album").join(format!("A - Song (Live) ({id}).mp3")), b"").unwrap();
        fs::write(root.join("Singles").join(format!("A - Song (Live) ({id}).flac")), b"").unwrap();
        fs::write(root.join("Album").join("notes.txt"), b"").unwrap();
        fs::write(root.join("Album").join(format!("A - Song ({id}).mp3.part")), b"").unwrap();

        remove_stale_parts(&root);
        assert!(!root.join("Album").join(format!("A - Song ({id}).mp3.part")).exists());
        let found = collect_file_occurences(&root, &strip_id_suffix);
        let _ = fs::remove_dir_all(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found["A - Song (Live)"].len(), 2);
    }

    #[test]
    fn strips_only_trailing_id() {
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        assert_eq!(
            strip_id_suffix(&format!("A - Song (Live) ({})", id)).as_deref(),
            Some("A - Song (Live)")
        );
        assert_eq!(strip_id_suffix("A - Song (Live)"), None);
        assert_eq!(strip_id_suffix("random file"), None);
        assert_eq!(split_id_suffix(&format!("A - Song ({})", id)).unwrap().1, id);
    }

    #[test]
    fn finds_a_track_id_in_nested_folders() {
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        let root = std::env::temp_dir().join(format!("lsd-fs-id-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Album")).unwrap();
        fs::write(root.join("Album").join("cover.jpg"), b"").unwrap();
        fs::write(root.join("Album").join(format!("A - Song ({id}).mp3.part")), b"").unwrap();
        assert_eq!(first_track_id(&root), None);
        fs::write(root.join("Album").join(format!("A - Song ({id}).mp3")), b"").unwrap();
        let found = first_track_id(&root);
        let _ = fs::remove_dir_all(&root);
        assert_eq!(found.as_deref(), Some(id));
    }
}
