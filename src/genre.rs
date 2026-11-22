use std::{fs, io, path::{Path, PathBuf}};
use log::{error, info, warn};
use sanitize_filename::sanitize;
use librespot::{core::SpotifyId, metadata::Track};
use crate::{config::{backup_config, save_config}, ctx::DLContext, download::GetById, fs::first_track_id};

/// What the user typed at a genre menu.
#[derive(Debug, PartialEq)]
pub enum Choice {
    Skip,
    Quit,
    Genre(String),
}

/// Interprets a menu answer: empty = skip, `q` = stop asking, a number picks from `options`, anything else is a new genre name.
pub fn parse_choice(input: &str, options: &[String]) -> Choice {
    let input = input.trim();
    if input.is_empty() {
        return Choice::Skip;
    }
    if input.eq_ignore_ascii_case("q") || input.eq_ignore_ascii_case("quit") {
        return Choice::Quit;
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

/// Records the genre in the config and saves it. Returns the genre it replaced (the default if none).
fn set_genre(ctx: &mut DLContext, b62id: &str, name: &str, genre: &str) -> String {
    let old_genre = crate::metadata::ArtistExt::get_genre(ctx, &b62id.to_string());
    ctx.config.artist_genres.insert(b62id.to_string(), vec![genre.to_string()]);
    ctx.config.artist_names.insert(b62id.to_string(), name.to_string());
    ctx.unsorted.remove(b62id);
    if let Err(e) = save_config(&ctx.config) {
        error!("Config Save Fail: {}", e);
    }
    old_genre
}

/// Moves an artist folder and fixes the file records. Returns false if the move failed.
fn refile(ctx: &mut DLContext, b62id: &str, name: &str, src: &Path, dst: &Path) -> bool {
    match move_dir_merge(src, dst) {
        Ok(conflicts) => {
            for file in ctx.filedb.files.values_mut().flatten() {
                if let Ok(rest) = file.path.strip_prefix(src) {
                    file.path = dst.join(rest);
                }
            }
            for c in conflicts {
                warn!("Move Conflict, left in place: {:?}", c);
            }
            if let Some(parent) = src.parent() {
                let _ = fs::remove_dir(parent); // drop an emptied genre folder
            }
            info!("<{}> Move \"{}\": {:?} -> {:?}", b62id, name, src, dst);
            true
        }
        Err(e) => {
            error!("<{}> Move Fail \"{}\": {}", b62id, name, e);
            false
        }
    }
}

/// Sets an artist's genre, saves the config, and re-files any existing downloads.
pub fn assign_genre(ctx: &mut DLContext, b62id: &str, name: &str, genre: &str) {
    let old_genre = set_genre(ctx, b62id, name, genre);
    if !old_genre.eq_ignore_ascii_case(genre) {
        let (src, dst) = (artist_dir(ctx, &old_genre, name), artist_dir(ctx, genre, name));
        refile(ctx, b62id, name, &src, &dst);
    }
}

/// Reads one answer to the genre menu. `None` means stdin hit EOF.
fn ask_genre(ctx: &DLContext, name: &str) -> Option<Choice> {
    let options = known_genres(ctx);
    let menu = options.iter().enumerate()
        .map(|(i, g)| format!("[{}] {}", i + 1, g))
        .collect::<Vec<_>>().join(" ");
    info!("Genre for \"{}\"? {} or type a new one, Enter = skip, q = stop: ", name, menu);
    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(parse_choice(&line, &options)),
    }
}

/// Offers a menu for every artist collected without a genre. Returns false on EOF.
pub fn prompt_unsorted(ctx: &mut DLContext) -> bool {
    let pending: Vec<(String, String)> = ctx.unsorted.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (id, name) in pending {
        match ask_genre(ctx, &name) {
            None => return false,
            Some(Choice::Quit) => break,
            Some(Choice::Skip) => {}
            Some(Choice::Genre(g)) => assign_genre(ctx, &id, &name, &g),
        }
    }
    true
}

/// Artist folders directly under `<root>/<default genre>`.
pub fn generic_artist_folders(ctx: &DLContext) -> Vec<PathBuf> {
    let dir = PathBuf::from(&ctx.config.root_folder).join(&ctx.config.default_genre);
    let mut folders: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten()
        .map(|e| e.path()).filter(|p| p.is_dir()).collect();
    folders.sort();
    folders
}

/// Album artist (id, name) of a downloaded track, looked up on Spotify.
async fn artist_of_track(ctx: &DLContext, track_id: &str) -> Result<(String, String), librespot::core::Error> {
    let track = Track::get_by_id(ctx, SpotifyId::from_base62(track_id)?).await?;
    let artist = track.album.artists.0.first()
        .ok_or_else(|| librespot::core::Error::not_found("track has no album artist"))?;
    Ok((artist.id.to_id()?, artist.name.clone()))
}

/// Walks the default-genre folder, works out each artist's id, and asks for a genre.
/// With `dry_run` nothing is written, moved or asked; it only reports what would happen.
/// Returns false on EOF.
pub async fn sort_generic(ctx: &mut DLContext, dry_run: bool) -> bool {
    let folders = generic_artist_folders(ctx);
    if folders.is_empty() {
        info!("Nothing to sort under \"{}/{}\".", ctx.config.root_folder, ctx.config.default_genre);
        return true;
    }
    if !dry_run {
        match backup_config() {
            Ok(Some(bak)) => info!("Config backed up to \"{}\"", bak),
            Ok(None) => {}
            Err(e) => {
                error!("Config backup failed, not sorting: {}", e);
                return true;
            }
        }
    }
    info!("{} artist folder(s){}", folders.len(), if dry_run { " (dry run, nothing will change)" } else { "" });
    let (mut moved, mut skipped, mut unresolved) = (0, 0, 0);
    for folder in folders {
        let folder_name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(track_id) = first_track_id(&folder) else {
            warn!("No downloaded tracks found, skipping \"{}\"", folder_name);
            unresolved += 1;
            continue;
        };
        let (b62id, name) = match artist_of_track(ctx, &track_id).await {
            Ok(found) => found,
            Err(e) => {
                warn!("Artist lookup failed for \"{}\": {}", folder_name, e);
                unresolved += 1;
                continue;
            }
        };
        let configured = ctx.config.artist_genres.get(&b62id).and_then(|v| v.first()).cloned();
        let target = match configured {
            Some(g) if !g.eq_ignore_ascii_case(&ctx.config.default_genre) => {
                info!("<{}> \"{}\" is already set to {}", b62id, folder_name, g);
                g
            }
            _ if dry_run => {
                info!("<{}> \"{}\" would be asked for a genre", b62id, folder_name);
                skipped += 1;
                continue;
            }
            _ => match ask_genre(ctx, &folder_name) {
                None => return false,
                Some(Choice::Quit) => break,
                Some(Choice::Skip) => {
                    skipped += 1;
                    continue;
                }
                Some(Choice::Genre(g)) => {
                    set_genre(ctx, &b62id, &name, &g);
                    g
                }
            },
        };
        let dst = PathBuf::from(&ctx.config.root_folder).join(&target).join(&folder_name);
        if dry_run {
            info!("<{}> would move \"{}\" -> {:?}", b62id, folder_name, dst);
            moved += 1;
        } else if target.eq_ignore_ascii_case(&ctx.config.default_genre) || refile(ctx, &b62id, &name, &folder, &dst) {
            moved += 1;
        }
    }
    info!("Sort done: {} {}, {} skipped, {} unresolved", moved, if dry_run { "would move" } else { "moved" }, skipped, unresolved);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_menu_answers() {
        let opts = vec!["Pop".to_string(), "Rock".to_string()];
        assert_eq!(parse_choice("", &opts), Choice::Skip);
        assert_eq!(parse_choice("Q", &opts), Choice::Quit);
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
