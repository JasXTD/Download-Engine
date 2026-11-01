//! Added by Download Engine. See ../../patch.md.
//!
//! Random values come straight from the operating system. Peer ids, DHT tokens and node
//! ids, and tracker transaction ids only need random bytes, so nothing else is required.

/// Fills `dest` with random bytes. Panics if the operating system can't provide them,
/// because a guessable token would be worse than stopping.
pub fn fill(dest: &mut [u8]) {
    getrandom::fill(dest).expect("the operating system could not provide random bytes");
}

pub fn random_u32() -> u32 {
    let mut bytes = [0u8; 4];
    fill(&mut bytes);
    u32::from_ne_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_each_call_gives_new_bytes() {
        let (mut first, mut second) = ([0u8; 32], [0u8; 32]);
        fill(&mut first);
        fill(&mut second);
        assert_ne!(first, [0u8; 32]);
        assert_ne!(first, second);
    }
}
