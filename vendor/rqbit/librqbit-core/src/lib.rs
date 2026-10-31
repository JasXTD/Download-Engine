// Modified by Download Engine: added safe_name, removed directories. See ../../patch.md.
pub mod compact_ip;
pub mod constants;
mod error;
pub mod hash_id;
pub mod lengths;
pub mod magnet;
pub mod peer_id;
pub mod safe_name;
pub mod spawn_utils;
pub mod speed_estimator;
pub mod torrent_metainfo;
pub use hash_id::{Id20, Id32};

pub use error::Error;
pub type Result<T> = std::result::Result<T, Error>;
