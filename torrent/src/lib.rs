//! Download Engine's torrent engine, built on the protocol crates in `vendor/rqbit`.

pub mod bitfield;
pub mod discovery;
pub mod meta;
mod metadata;
pub mod peer;
pub mod picker;
pub mod rate;
mod resume;
pub mod session;
pub mod storage;
pub mod torrent;

#[cfg(test)]
mod testkit;
#[cfg(test)]
mod transfer_tests;

/// Whether to write diagnostics to stderr, for looking into a failed test run.
pub fn debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("DE_TORRENT_DEBUG").is_some())
}

#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if $crate::debug_enabled() {
            eprintln!($($arg)*);
        }
    };
}
