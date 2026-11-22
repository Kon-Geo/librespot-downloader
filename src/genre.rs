use std::{fs, io, path::{Path, PathBuf}};
use log::{error, info, warn};
use sanitize_filename::sanitize;
use crate::{config::save_config, ctx::DLContext};

/// What the user typed at a genre menu.
#[derive(Debug, PartialEq)]
pub enum Choice {
    Skip,
    Genre(String),
}

/// Interprets a menu answer: empty = skip, a number picks from `options`, anything else is a new genre name.
pub fn parse_choice(input: &str, options: &[String]) -> Choice {
    let input = input.trim();
    if input.is_empty() {
        return Choice::Skip;
    }
    if let Ok(n) = input.parse::<usize>() {
        return match options.get(n.wrapping_sub(1)) {
            Some(g) => Choice::Genre(g.clone()),
            None => Choice::Skip,
        };
    }
    Choice::Genre(sanitize(input))
}

/// Genres already in use, plus the default, in stable order.
pub fn known_genres(ctx: &DLContext) -> Vec<String> {
    let mut genres: Vec<String> = ctx.config.artist_genres.values()
        .filter_map(|v| v.first().cloned())
        .collect();
    genres.push(ctx.config.default_genre.clone());
    genres.sort_by_key(|g| g.to_lowercase());
    genres.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    genres
}

/// Moves `src` into `dst`, merging directories. Files that would overwrite an existing
/// file are left in place and returned. Empty source directories are removed.
pub fn move_dir_merge(src: &Path, dst: &Path) -> io::Result<Vec<PathBuf>> {
    let mut conflicts = Vec::new();
    if !src.exists() {
        return Ok(conflicts);
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            conflicts.extend(move_dir_merge(&from, &to)?);
        } else if to.exists() {
            conflicts.push(from);
        } else {
            fs::rename(&from, &to)?;
        }
    }
    let _ = fs::remove_dir(src); // only succeeds when empty
    Ok(conflicts)
}

fn artist_dir(ctx: &DLContext, genre: &str, name: &str) -> PathBuf {
    PathBuf::from(&ctx.config.root_folder).join(genre).join(sanitize(name))
}

/// Sets an artist's genre, saves the config, and re-files any existing downloads.
pub fn assign_genre(ctx: &mut DLContext, b62id: &str, name: &str, genre: &str) {
    let old_genre = crate::metadata::ArtistExt::get_genre(ctx, &b62id.to_string());
    ctx.config.artist_genres.insert(b62id.to_string(), vec![genre.to_string()]);
    ctx.config.artist_names.insert(b62id.to_string(), name.to_string());
    ctx.unsorted.remove(b62id);
    if let Err(e) = save_config(&ctx.config) {
        error!("Config Save Fail: {}", e);
    }
    if old_genre.eq_ignore_ascii_case(genre) {
        return;
    }
    let src = artist_dir(ctx, &old_genre, name);
    let dst = artist_dir(ctx, genre, name);
    match move_dir_merge(&src, &dst) {
        Ok(conflicts) => {
            for file in ctx.filedb.files.values_mut().flatten() {
                if let Ok(rest) = file.path.strip_prefix(&src) {
                    file.path = dst.join(rest);
                }
            }
            for c in conflicts {
                warn!("Move Conflict, left in place: {:?}", c);
            }
            if src.parent().map_or(false, |p| p.exists()) {
                let _ = fs::remove_dir(src.parent().unwrap()); // drop emptied genre folder
            }
            info!("<{}> Genre \"{}\" -> {}", b62id, name, genre);
        }
        Err(e) => error!("<{}> Move Fail \"{}\": {}", b62id, name, e),
    }
}

/// Offers a menu for every artist collected without a genre. Returns false on EOF.
pub fn prompt_unsorted(ctx: &mut DLContext) -> bool {
    let pending: Vec<(String, String)> = ctx.unsorted.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (id, name) in pending {
        let options = known_genres(ctx);
        let menu = options.iter().enumerate()
            .map(|(i, g)| format!("[{}] {}", i + 1, g))
            .collect::<Vec<_>>().join(" ");
        info!("Genre for \"{}\"? {} or type a new one, Enter = skip: ", name, menu);
        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => return false,
            Ok(_) => {}
        }
        if let Choice::Genre(g) = parse_choice(&line, &options) {
            assign_genre(ctx, &id, &name, &g);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_menu_answers() {
        let opts = vec!["Pop".to_string(), "Rock".to_string()];
        assert_eq!(parse_choice("", &opts), Choice::Skip);
        assert_eq!(parse_choice("2", &opts), Choice::Genre("Rock".into()));
        assert_eq!(parse_choice("9", &opts), Choice::Skip);
        assert_eq!(parse_choice("0", &opts), Choice::Skip);
        assert_eq!(parse_choice(" Jazz \n", &opts), Choice::Genre("Jazz".into()));
    }

    #[test]
    fn merges_directories_and_reports_conflicts() {
        let root = std::env::temp_dir().join(format!("lsd-genre-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (src, dst) = (root.join("Generic/A"), root.join("Rock/A"));
        fs::create_dir_all(src.join("Album")).unwrap();
        fs::create_dir_all(dst.join("Album")).unwrap();
        fs::write(src.join("Album/one.mp3"), b"new").unwrap();
        fs::write(src.join("Album/two.mp3"), b"new").unwrap();
        fs::write(dst.join("Album/two.mp3"), b"old").unwrap();
        let conflicts = move_dir_merge(&src, &dst).unwrap();
        let moved = dst.join("Album/one.mp3").exists();
        let kept = fs::read(dst.join("Album/two.mp3")).unwrap();
        let _ = fs::remove_dir_all(&root);
        assert!(moved);
        assert_eq!(kept, b"old");
        assert_eq!(conflicts.len(), 1);
    }
}
