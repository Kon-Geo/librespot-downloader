use std::{collections::HashMap, fs, path::{Path, PathBuf}};
use crate::{metadata::{ArtistExt, TrackFileDescriptor}};

pub type FileOccurences = HashMap<String, Vec<PathBuf>>;

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
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(stem) = filter(stem) else {
            continue;
        };
        occurrences.entry(stem).or_default().push(path);
    }
    occurrences
}

pub fn remove_bracketed_content(input: &str) -> Option<String> {
    let mut result = String::new();
    let mut depth = 0;
    for c in input.chars() {
        match c {
            '(' | '[' => {
                depth += 1;
            }
            ')' | ']' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {
                if depth == 0 {
                    result.push(c);
                }
            }
        }
    }
    Some(result)
}

pub struct FileDB {
    files: FileOccurences,
    tracked_artists: Vec<String>,
}

impl FileDB {
    pub fn new() -> Self {
        Self { files: FileOccurences::new(), tracked_artists: Vec::new() }
    }

    pub fn get(&self, stem: &String) -> Option<&Vec<PathBuf>> {
        self.files.get(stem)
    }

    pub fn track_artist(&mut self, artist: &ArtistExt) {
        if self.tracked_artists.contains(&artist.b62id) {
            return;
        }
        let occurences = collect_file_occurences(&artist.folder, &remove_bracketed_content);
        self.files.extend(occurences);
        self.tracked_artists.push(artist.b62id.clone());
    }

    pub fn track_track(&mut self, file: &TrackFileDescriptor) {
        self.files
            .entry(file.stem.clone())
            .or_default()
            .push(file.path.clone());
    }
}

