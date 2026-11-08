//! Download Engine's torrent engine, built on the protocol crates in `vendor/rqbit`.

mod admission;
pub mod bitfield;
pub mod cli;
pub mod details;
pub mod discovery;
pub mod meta;
mod metadata;
pub mod mse;
pub mod network;
pub mod peer;
pub mod picker;
pub mod rate;
pub mod reason;
mod resume;
pub mod router;
pub mod session;
pub mod storage;
pub mod torrent;
pub mod transport;
pub mod webseed;

#[cfg(test)]
mod details_tests;
#[cfg(test)]
mod hard_tests;
#[cfg(all(test, unix))]
mod network_binding_tests;
#[cfg(all(test, unix))]
mod network_tests;
#[cfg(test)]
mod testkit;
#[cfg(test)]
mod transfer_tests;
#[cfg(all(test, unix))]
mod transport_tests;

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
