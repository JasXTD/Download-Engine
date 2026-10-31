//! The torrent engine. The app starts this only while a torrent is active; see `src/torrent`.

fn main() -> anyhow::Result<()> {
    de_torrent::cli::run()
}
