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

- `artist_genres`: artist id -> genres; first entry is the folder and genre tag.
- `exclude`: album/track ids to skip.
