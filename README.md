# librespot-downloader

Downloads Spotify tracks, albums, artists and playlists with [librespot](https://github.com/librespot-org/librespot)
(requires a Premium account), tags them and files them as `root/<genre>/<artist>/<album>/`.

## Usage

    cargo run --release

On first run a browser window opens for OAuth login; credentials are cached in `.cache/`.
Then paste Spotify URLs (several per line is fine) or `q` to quit. Accepted forms:
`https://open.spotify.com/{track,album,artist,playlist}/ID`, `intl-xx/` and `user/<name>/` variants,
and `spotify:track:ID` URIs.

## Behaviour

- Best available format is chosen (FLAC > AAC 320 > MP3 > Vorbis ...).
- Files are named `Artist - Title (<id>).ext`. Existing files are detected by that name, so re-running skips them.
- A track found both in an album and as a single keeps the album copy; a better format replaces a worse one.
- Downloads are written to `.part` files and renamed when finished.

## Config

Copy `config.example.json` to `config.json` (git-ignored). A missing file means defaults; a malformed one logs a warning.

- `artist_genres`: artist id -> genres; first entry is the folder name (genre is not written into file tags). You normally don't edit this by hand (see below).
- `artist_names`: optional artist id -> name, filled in automatically so the file stays readable.
- `exclude`: album/track ids to skip.

## Assigning genres

After each batch, any artist without a genre is offered a menu (genres already in use, or type a new one, Enter to skip).
The answer is saved to `config.json` and the artist's existing folder is moved under the new genre.

- `genre <artist url> <Genre>`: set a genre directly (also re-files that artist's folder).
- `genres`: be prompted again for artists you skipped.
