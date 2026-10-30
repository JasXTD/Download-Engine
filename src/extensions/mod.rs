//! Extensions: packages of Lua jobs a person installs rather than writes.
//!
//! A package groups jobs. It never merges their powers: each job is saved as an
//! ordinary script and runs in its own worker with the limits of its kind. Jobs
//! share only read-only settings, and Rust limits them to the hosts they declare.
mod package;
mod runtime;
mod store;
#[cfg(test)]
mod tests;

pub use package::{
    FILE_SUFFIX, FORMAT, FORMAT_VERSION, Job, MAX_CODE_BYTES, MAX_FILES, MAX_HOSTS, MAX_JOBS,
    MAX_PACKAGE_BYTES, MAX_SETTINGS, Package, Permissions, Setting, host_allowed, pages_overlap,
    url_allowed, valid_path, version,
};
pub use runtime::{Context, Reach};
pub(crate) use store::initialize;
pub use store::{Changes, Grants, Installed, Review, pack};
