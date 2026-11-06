//! Errors the app recognises by a code, so that it can explain them in the person's own
//! language. The English message stays for logs and for anyone driving the engine by hand.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reason {
    pub code: &'static str,
    pub message: &'static str,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for Reason {}

/// A magnet with no usable tracker, saved details or given peer. Only a DHT search could
/// find it, and that tells strangers which torrent this computer wants.
pub const NEEDS_DHT_CONSENT: Reason = Reason {
    code: "needs_dht_consent",
    message: "This magnet has no usable tracker or cached metadata. Open its .torrent file to keep a potentially private hash off the public DHT.",
};

/// As above, but the DHT is switched off, so not even a yes could find it.
pub const DHT_OFF: Reason = Reason {
    code: "dht_off",
    message: "This magnet has no usable tracker or cached metadata, and finding peers without a tracker (DHT) is off. Open its .torrent file instead.",
};

/// A DHT search found the details, and they say the torrent is private. Private torrents
/// may only use their own site's trackers.
pub const PRIVATE_FROM_DHT: Reason = Reason {
    code: "private_from_dht",
    message: "This is a private torrent, so it may only use its own site's tracker. Open the .torrent file from that site.",
};

/// Nobody answered a DHT search with the details.
pub const DHT_SEARCH_TIMEOUT: Reason = Reason {
    code: "dht_search_timeout",
    message: "No one shared this torrent's file list. The DHT search stopped after 10 minutes.",
};

/// The code of an error, when it is one of these, however much context was added on top.
pub fn code_of(error: &anyhow::Error) -> Option<&'static str> {
    error.downcast_ref::<Reason>().map(|reason| reason.code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;

    #[test]
    fn a_code_survives_added_context_and_other_errors_have_none() {
        let error = Err::<(), _>(anyhow::Error::from(DHT_OFF))
            .context("while adding a magnet")
            .unwrap_err();
        assert_eq!(code_of(&error), Some("dht_off"));
        assert!(format!("{error:#}").contains("DHT"));
        assert_eq!(code_of(&anyhow::anyhow!("something else")), None);
    }
}
