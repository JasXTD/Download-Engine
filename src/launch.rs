//! Shell activation arguments are data, not additional app commands. In
//! particular, --open consumes exactly one argument and cannot mix with helpers.
use anyhow::{Result, ensure};
use download_engine::{
    external_open::{MAX_PENDING, Target},
    store::Store,
};
use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Default, Debug)]
pub struct Launch {
    pub browser_setup: bool,
    pub autostart: bool,
    pub targets: Vec<Target>,
}
impl Launch {
    pub fn parse(args: &[OsString], base: &Path) -> Result<Self> {
        let mut launch = Self::default();
        if args.first().is_some_and(|a| a == "--open") {
            ensure!(
                args.len() == 2,
                "Expected one .torrent file or magnet link after --open"
            );
            launch.targets.push(Target::parse(&args[1], base)?);
        } else {
            for arg in args {
                if arg == crate::startup::ARGUMENT {
                    launch.autostart = true;
                } else if arg == "--browser-setup" {
                    launch.browser_setup = true;
                } else {
                    launch.targets.push(Target::parse(arg, base)?);
                }
            }
            ensure!(
                launch.targets.is_empty() || (!launch.autostart && !launch.browser_setup),
                "Do not combine torrent inputs with startup options"
            );
        }
        ensure!(
            launch.targets.len() <= MAX_PENDING,
            "Open at most eight torrents at a time"
        );
        Ok(launch)
    }
    pub fn forward(&self, store: &Store) -> Result<()> {
        let submitted = store.submit_external_opens(&self.targets)?;
        let result = (|| -> Result<()> {
            // Payloads remain in the local profile inbox; no URLs or paths cross
            // the existing untrusted Win32 show-window message.
            crate::tray::show_existing(&store.paths, false)?;
            let deadline = Instant::now() + Duration::from_secs(8);
            while Instant::now() < deadline {
                if store.external_opens_received(&submitted)? {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            anyhow::bail!(
                "The running Download Engine did not accept this torrent. Close any older copy, then open the torrent again."
            )
        })();
        if result.is_err() && store.withdraw_unreceived_external_opens(&submitted)? {
            return Ok(());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Launch> {
        let root = std::env::current_dir().unwrap();
        Launch::parse(&args.iter().map(OsString::from).collect::<Vec<_>>(), &root)
    }
    #[test]
    fn shell_input_cannot_inject_helper_or_startup_arguments() {
        for args in [
            vec!["--open"],
            vec!["--open", "a.torrent", "--browser-setup"],
            vec!["--open", "--antivirus-scan-request"],
            vec!["a.torrent", "--browser-setup"],
            vec!["--unknown"],
            vec!["--open", "file:///C:/a.torrent"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
    #[test]
    fn explorer_unicode_paths_multiple_files_and_existing_flags_work() {
        assert_eq!(
            parse(&["--open", "文件 with spaces.torrent"])
                .unwrap()
                .targets
                .len(),
            1
        );
        assert_eq!(
            parse(&["one.torrent", "two.torrent"])
                .unwrap()
                .targets
                .len(),
            2
        );
        assert!(parse(&[crate::startup::ARGUMENT]).unwrap().autostart);
        assert!(parse(&["--browser-setup"]).unwrap().browser_setup);
        assert!(parse(&[]).unwrap().targets.is_empty());
        assert!(parse(&["a.torrent"; 9]).is_err());
    }
    #[test]
    fn running_instance_acknowledges_review_without_adding_a_download() {
        let dir = tempfile::tempdir().unwrap();
        let paths = download_engine::paths::AppPaths {
            data: dir.path().join("data"),
            downloads: dir.path().join("downloads"),
        };
        let receiver = Store::open(paths.clone()).unwrap();
        let sender = Store::open(paths).unwrap();
        let worker = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let inbox = receiver.receive_external_opens().unwrap();
                if inbox.count > 0 {
                    assert_eq!(inbox.count, 1);
                    assert!(receiver.list().unwrap().is_empty());
                    return;
                }
                assert!(Instant::now() < deadline, "sender did not deliver");
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let launch = Launch::parse(
            &[OsString::from("--open"), OsString::from("sample.torrent")],
            dir.path(),
        )
        .unwrap();
        launch.forward(&sender).unwrap();
        worker.join().unwrap();
        assert_eq!(sender.receive_external_opens().unwrap().count, 1);
        assert!(sender.list().unwrap().is_empty());
    }
}
