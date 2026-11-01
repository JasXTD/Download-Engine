//! The `de-torrent` program. The app builds the same program as `src/bin/de-torrent.rs`.

fn main() -> anyhow::Result<()> {
    de_torrent::cli::run()
}
