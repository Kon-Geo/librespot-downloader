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

/// Splits "<base> (<22-char base62 id>)" into `<base>`. Returns None for files
/// that don't follow this downloader's naming scheme.
pub fn strip_id_suffix(stem: &str) -> Option<String> {
    let inner = stem.strip_suffix(')')?;
    let open = inner.rfind(" (")?;
    let id = &inner[open + 2..];
    if id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(inner[..open].to_string())
    } else {
        None
    }
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
        let occurrences = collect_file_occurences(folder, &strip_id_suffix);
        self.files.extend(occurrences);
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
    use super::strip_id_suffix;

    #[test]
    fn strips_only_trailing_id() {
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        assert_eq!(
            strip_id_suffix(&format!("A - Song (Live) ({})", id)).as_deref(),
            Some("A - Song (Live)")
        );
        assert_eq!(strip_id_suffix("A - Song (Live)"), None);
        assert_eq!(strip_id_suffix("random file"), None);
    }
}
