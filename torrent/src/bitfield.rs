//! A set of piece numbers, stored the way the peer protocol sends it: piece 0 is the high
//! bit of the first byte.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitfield {
    bytes: Vec<u8>,
    len: u32,
    ones: u32,
}

impl Bitfield {
    pub fn new(len: u32) -> Self {
        Self {
            bytes: vec![0; len.div_ceil(8) as usize],
            len,
            ones: 0,
        }
    }

    pub fn full(len: u32) -> Self {
        let mut field = Self::new(len);
        for index in 0..len {
            field.set(index);
        }
        field
    }

    /// Reads a bitfield from a peer. The size must be exact, and spare bits at the end
    /// are cleared, because they would otherwise count as pieces that don't exist.
    pub fn from_wire(bytes: &[u8], len: u32) -> Option<Self> {
        if bytes.len() != len.div_ceil(8) as usize {
            return None;
        }
        let mut bytes = bytes.to_vec();
        let spare = (8 - len % 8) % 8;
        if let Some(last) = bytes.last_mut() {
            *last &= 0xffu8 << spare;
        }
        let ones = bytes.iter().map(|byte| byte.count_ones()).sum();
        Some(Self { bytes, len, ones })
    }

    pub fn get(&self, index: u32) -> bool {
        index < self.len && self.bytes[(index / 8) as usize] & (0x80 >> (index % 8)) != 0
    }

    /// Returns true when the bit was not set before.
    pub fn set(&mut self, index: u32) -> bool {
        if index >= self.len || self.get(index) {
            return false;
        }
        self.bytes[(index / 8) as usize] |= 0x80 >> (index % 8);
        self.ones += 1;
        true
    }

    pub fn clear(&mut self, index: u32) {
        if self.get(index) {
            self.bytes[(index / 8) as usize] &= !(0x80 >> (index % 8));
            self.ones -= 1;
        }
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn count(&self) -> u32 {
        self.ones
    }

    pub fn is_full(&self) -> bool {
        self.ones == self.len
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn iter_set(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.len).filter(|index| self.get(*index))
    }

    /// Whether any piece is in `self`, in `wanted`, and missing from `have`. Works a byte
    /// at a time, because this runs on every message that can change a peer's usefulness.
    pub fn has_any_needed(&self, wanted: &Bitfield, have: &Bitfield) -> bool {
        self.bytes
            .iter()
            .zip(&wanted.bytes)
            .zip(&have.bytes)
            .any(|((theirs, wanted), have)| theirs & wanted & !have != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_piece_is_the_high_bit() {
        let mut field = Bitfield::new(10);
        assert!(field.set(0));
        assert!(field.set(9));
        assert_eq!(field.as_bytes(), [0x80, 0x40]);
    }

    #[test]
    fn test_setting_twice_counts_once() {
        let mut field = Bitfield::new(8);
        assert!(field.set(3));
        assert!(!field.set(3));
        assert_eq!(field.count(), 1);
        field.clear(3);
        field.clear(3);
        assert_eq!(field.count(), 0);
    }

    #[test]
    fn test_numbers_past_the_end_are_ignored() {
        let mut field = Bitfield::new(9);
        assert!(!field.set(9));
        assert!(!field.get(9));
        assert!(!field.get(u32::MAX));
        assert_eq!(field.count(), 0);
    }

    #[test]
    fn test_wire_size_must_be_exact() {
        assert!(Bitfield::from_wire(&[0xff], 9).is_none());
        assert!(Bitfield::from_wire(&[0xff, 0xff, 0xff], 9).is_none());
        assert!(Bitfield::from_wire(&[0xff, 0x80], 9).is_some());
    }

    #[test]
    fn test_spare_bits_from_a_peer_do_not_count() {
        let field = Bitfield::from_wire(&[0xff, 0xff], 9).unwrap();
        assert_eq!(field.count(), 9);
        assert!(field.is_full());
        assert_eq!(field.as_bytes(), [0xff, 0x80]);
    }

    #[test]
    fn test_a_whole_number_of_bytes_has_no_spare_bits() {
        let field = Bitfield::from_wire(&[0xff], 8).unwrap();
        assert_eq!(field.count(), 8);
    }

    #[test]
    fn test_full_and_iteration() {
        let field = Bitfield::full(3);
        assert!(field.is_full());
        assert_eq!(field.iter_set().collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn test_needed_means_theirs_wanted_and_missing() {
        let mut theirs = Bitfield::new(16);
        let mut wanted = Bitfield::new(16);
        let mut have = Bitfield::new(16);
        theirs.set(4);
        assert!(!theirs.has_any_needed(&wanted, &have));
        wanted.set(4);
        assert!(theirs.has_any_needed(&wanted, &have));
        have.set(4);
        assert!(!theirs.has_any_needed(&wanted, &have));
    }
}
