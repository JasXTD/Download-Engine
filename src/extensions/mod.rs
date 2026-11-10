//! Extensions: packages of Lua jobs a person installs rather than writes.
//!
//! A package groups jobs. It never merges their powers: each job is saved as an
//! ordinary script and runs in its own worker with the limits of its kind. Jobs
//! share settings, and Rust limits them to the hosts they declare. Explicit web
//! actions may save only the protected text settings named in their install review.
mod actions;
#[cfg(test)]
mod debrid_tests;
pub mod discovery;
#[cfg(test)]
mod example_tests;
pub mod examples;
mod package;
mod runtime;
mod store;
#[cfg(test)]
mod tests;

pub use package::{
    ActionInput, FILE_SUFFIX, FORMAT, FORMAT_VERSION, Job, MAX_CODE_BYTES, MAX_FILES, MAX_HOSTS,
    MAX_JOBS, MAX_PACKAGE_BYTES, MAX_SETTINGS, Package, Permissions, Setting, host_allowed,
    pages_overlap, url_allowed, valid_path, version,
};
pub use runtime::{Context, Reach};
pub(crate) use store::initialize;
pub use store::{Changes, Grants, Installed, Review, pack};
