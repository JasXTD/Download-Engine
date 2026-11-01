// Modified by Download Engine: hashes with sha1 and sha2, removed the aws-lc backend. See ../../patch.md.
// Wrapper for sha1/sha256 libraries to be able to swap them easily,
// e.g. to measure performance, or change implementations depending on platform.
//
// Sha1 computation is the majority of CPU usage of librqbit.

use sha1::Digest;

pub trait ISha1 {
    fn new() -> Self;
    fn update(&mut self, buf: &[u8]);
    fn finish(self) -> [u8; 20];
}

/// SHA-256 hash trait for BEP 52 (BitTorrent v2) support.
pub trait ISha256 {
    fn new() -> Self;
    fn update(&mut self, buf: &[u8]);
    fn finish(self) -> [u8; 32];

    fn finish_id32(self) -> [u8; 32]
    where
        Self: Sized,
    {
        self.finish()
    }
}

pub struct Sha1(sha1::Sha1);

impl ISha1 for Sha1 {
    fn new() -> Self {
        Self(Digest::new())
    }

    fn update(&mut self, buf: &[u8]) {
        Digest::update(&mut self.0, buf);
    }

    fn finish(self) -> [u8; 20] {
        self.0.finalize().into()
    }
}

pub struct Sha256(sha2::Sha256);

impl ISha256 for Sha256 {
    fn new() -> Self {
        Self(Digest::new())
    }

    fn update(&mut self, buf: &[u8]) {
        Digest::update(&mut self.0, buf);
    }

    fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::{ISha1, ISha256, Sha1, Sha256};

    const TWO_BLOCKS: &[u8] = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn sha1(input: &[u8]) -> String {
        let mut h = Sha1::new();
        h.update(input);
        hex(&h.finish())
    }

    fn sha256(input: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(input);
        hex(&h.finish())
    }

    #[test]
    fn test_sha1_known_vectors() {
        assert_eq!(sha1(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sha1(TWO_BLOCKS), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
    }

    #[test]
    fn test_sha256_known_vectors() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256(TWO_BLOCKS),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn test_input_can_arrive_in_many_small_updates() {
        // The published values for one million "a" characters.
        let mut one = Sha1::new();
        let mut two = Sha256::new();
        for _ in 0..1000 {
            one.update(&[b'a'; 1000]);
            two.update(&[b'a'; 1000]);
        }
        assert_eq!(
            hex(&one.finish()),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
        assert_eq!(
            hex(&two.finish()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    fn assert_sha256_impl<T: ISha256>() {}

    #[test]
    fn test_sha256_known_vector_empty() {
        assert_sha256_impl::<Sha256>();
        let mut h = Sha256::new();
        h.update(b"");
        let got = h.finish();
        let expected: [u8; 32] = [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
            0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
            0x78, 0x52, 0xb8, 0x55,
        ];
        assert_eq!(got, expected);
    }
}
