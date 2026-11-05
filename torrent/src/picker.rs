//! Decides which piece and chunk to ask each peer for. This is pure logic with no network
//! or disk access, so every rule can be tested on its own.
//!
//! A chunk moves from missing to requested to written. When every chunk of a piece is
//! written the piece is hashed, and only a piece that passes counts as ours. Rarest pieces
//! go first, so that what few peers hold spreads through the swarm before they leave.

use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use librqbit_core::lengths::Lengths;

use crate::bitfield::Bitfield;

/// A peer asks for at most this many copies of a chunk during the last stretch of a
/// download. More would waste bandwidth, and fewer would let one slow peer hold up the end.
const MAX_ENDGAME_COPIES: u8 = 3;

/// Rebuilding the rarest-first order is the only slow step here, so it happens at most this
/// often. A slightly old order still picks well.
const ORDER_REFRESH: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chunk {
    pub piece: u32,
    pub index: u32,
}

/// What to send a peer: which bytes of which piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkRequest {
    pub chunk: Chunk,
    pub begin: u32,
    pub length: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChunkState {
    Missing,
    /// How many peers were asked for it.
    Requested(u8),
    Written,
}

struct PartialPiece {
    chunks: Vec<ChunkState>,
    written: u32,
    /// Peers that sent part of this piece, to blame if its hash is wrong.
    contributors: HashSet<u64>,
}

impl PartialPiece {
    fn new(chunks: u32) -> Self {
        Self {
            chunks: vec![ChunkState::Missing; chunks as usize],
            written: 0,
            contributors: HashSet::new(),
        }
    }
}

/// The result of finishing a chunk.
#[derive(Debug, PartialEq, Eq)]
pub enum Written {
    /// Another peer got there first, or the piece is already done.
    Duplicate,
    Stored,
    /// That was the last chunk, so the piece is ready to hash.
    PieceComplete,
}

pub struct Picker {
    lengths: Lengths,
    have: Bitfield,
    wanted: Bitfield,
    /// Wanted pieces not yet ours. Kept up to date, because this is checked for each piece.
    remaining: u32,
    availability: Vec<u32>,
    partial: BTreeMap<u32, PartialPiece>,
    verifying: HashSet<u32>,
    order: Vec<u32>,
    order_dirty: bool,
    order_built: Option<Instant>,
    random: u64,
}

fn count_remaining(have: &Bitfield, wanted: &Bitfield) -> u32 {
    wanted.iter_set().filter(|piece| !have.get(*piece)).count() as u32
}

impl Picker {
    pub fn new(lengths: Lengths, have: Bitfield, wanted: Bitfield, seed: u64) -> Self {
        let pieces = lengths.total_pieces() as usize;
        let remaining = count_remaining(&have, &wanted);
        Self {
            lengths,
            have,
            wanted,
            remaining,
            availability: vec![0; pieces],
            partial: BTreeMap::new(),
            verifying: HashSet::new(),
            order: Vec::new(),
            order_dirty: true,
            order_built: None,
            random: seed | 1,
        }
    }

    pub fn have(&self) -> &Bitfield {
        &self.have
    }

    pub fn wanted(&self) -> &Bitfield {
        &self.wanted
    }

    pub fn is_finished(&self) -> bool {
        self.remaining == 0
    }

    /// Changes which pieces to download. Pieces no longer wanted keep their written chunks,
    /// so asking for them again later loses nothing.
    pub fn set_wanted(&mut self, wanted: Bitfield) {
        self.remaining = count_remaining(&self.have, &wanted);
        self.wanted = wanted;
        self.order_dirty = true;
        self.order_built = None;
    }

    pub fn peer_added(&mut self, peer_has: &Bitfield) {
        for piece in peer_has.iter_set() {
            self.availability[piece as usize] += 1;
        }
        self.order_dirty = true;
    }

    pub fn peer_removed(&mut self, peer_has: &Bitfield) {
        for piece in peer_has.iter_set() {
            self.availability[piece as usize] = self.availability[piece as usize].saturating_sub(1);
        }
        self.order_dirty = true;
    }

    pub fn peer_has_piece(&mut self, piece: u32) {
        if let Some(count) = self.availability.get_mut(piece as usize) {
            *count += 1;
            self.order_dirty = true;
        }
    }

    /// Whether this peer holds anything still needed.
    pub fn is_interesting(&self, peer_has: &Bitfield) -> bool {
        peer_has.has_any_needed(&self.wanted, &self.have)
    }

    fn is_needed(&self, piece: u32) -> bool {
        self.wanted.get(piece) && !self.have.get(piece) && !self.verifying.contains(&piece)
    }

    fn next_random(&mut self) -> u64 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        self.random
    }

    fn refresh_order(&mut self, now: Instant) {
        let stale = self
            .order_built
            .is_none_or(|built| now.duration_since(built) >= ORDER_REFRESH);
        if !self.order_dirty || !stale {
            return;
        }
        let pieces: Vec<u32> = self
            .wanted
            .iter_set()
            .filter(|piece| self.is_needed(*piece) && !self.partial.contains_key(piece))
            .collect();
        // The random tag spreads peers across equally rare pieces.
        let mut tagged: Vec<(u32, u64, u32)> = pieces
            .into_iter()
            .map(|piece| {
                let tag = self.next_random();
                (self.availability[piece as usize], tag, piece)
            })
            .collect();
        tagged.sort_unstable();
        self.order = tagged.into_iter().map(|(_, _, piece)| piece).collect();
        self.order_dirty = false;
        self.order_built = Some(now);
    }

    fn request_for(&self, piece: u32, index: u32) -> Option<ChunkRequest> {
        let valid = self.lengths.validate_piece_index(piece)?;
        Some(ChunkRequest {
            chunk: Chunk { piece, index },
            begin: self.lengths.chunk_offset_in_piece(valid, index)?,
            length: self.lengths.chunk_size(valid, index)?,
        })
    }

    fn chunks_in(&self, piece: u32) -> u32 {
        self.lengths
            .validate_piece_index(piece)
            .map_or(0, |valid| self.lengths.chunks_per_piece(valid))
    }

    /// Picks up to `limit` chunks for a peer. Started pieces come first, so that pieces
    /// finish instead of many sitting half done. Then new pieces, rarest first. Only when
    /// nothing is left unasked are chunks already out with other peers requested again.
    pub fn next_requests(
        &mut self,
        peer_has: &Bitfield,
        in_flight: &dyn Fn(Chunk) -> bool,
        limit: usize,
        now: Instant,
    ) -> Vec<ChunkRequest> {
        let mut picked = Vec::with_capacity(limit);
        self.take_from_started(peer_has, limit, &mut picked);
        if picked.len() < limit {
            self.take_from_new(peer_has, limit, &mut picked, now);
        }
        if picked.is_empty() {
            self.take_endgame(peer_has, in_flight, limit, &mut picked);
        }
        picked
    }

    fn take_from_started(
        &mut self,
        peer_has: &Bitfield,
        limit: usize,
        picked: &mut Vec<ChunkRequest>,
    ) {
        let pieces: Vec<u32> = self
            .partial
            .keys()
            .copied()
            .filter(|piece| peer_has.get(*piece) && self.is_needed(*piece))
            .collect();
        for piece in pieces {
            for index in 0..self.chunks_in(piece) {
                if picked.len() >= limit {
                    return;
                }
                let state = self.partial[&piece].chunks[index as usize];
                if state == ChunkState::Missing
                    && let Some(request) = self.request_for(piece, index)
                {
                    self.mark_requested(request.chunk);
                    picked.push(request);
                }
            }
        }
    }

    fn take_from_new(
        &mut self,
        peer_has: &Bitfield,
        limit: usize,
        picked: &mut Vec<ChunkRequest>,
        now: Instant,
    ) {
        self.refresh_order(now);
        let mut position = 0;
        while picked.len() < limit && position < self.order.len() {
            let piece = self.order[position];
            position += 1;
            if !peer_has.get(piece) || !self.is_needed(piece) || self.partial.contains_key(&piece) {
                continue;
            }
            let chunks = self.chunks_in(piece);
            self.partial.insert(piece, PartialPiece::new(chunks));
            for index in 0..chunks {
                if picked.len() >= limit {
                    break;
                }
                if let Some(request) = self.request_for(piece, index) {
                    self.mark_requested(request.chunk);
                    picked.push(request);
                }
            }
        }
    }

    fn take_endgame(
        &mut self,
        peer_has: &Bitfield,
        in_flight: &dyn Fn(Chunk) -> bool,
        limit: usize,
        picked: &mut Vec<ChunkRequest>,
    ) {
        let pieces: Vec<u32> = self
            .partial
            .keys()
            .copied()
            .filter(|piece| peer_has.get(*piece) && self.is_needed(*piece))
            .collect();
        for piece in pieces {
            for index in 0..self.chunks_in(piece) {
                if picked.len() >= limit {
                    return;
                }
                let chunk = Chunk { piece, index };
                let state = self.partial[&piece].chunks[index as usize];
                if let ChunkState::Requested(copies) = state
                    && copies < MAX_ENDGAME_COPIES
                    && !in_flight(chunk)
                    && let Some(request) = self.request_for(piece, index)
                {
                    self.mark_requested(chunk);
                    picked.push(request);
                }
            }
        }
    }

    fn mark_requested(&mut self, chunk: Chunk) {
        if let Some(partial) = self.partial.get_mut(&chunk.piece) {
            let state = &mut partial.chunks[chunk.index as usize];
            *state = match *state {
                ChunkState::Missing => ChunkState::Requested(1),
                ChunkState::Requested(copies) => ChunkState::Requested(copies.saturating_add(1)),
                ChunkState::Written => ChunkState::Written,
            };
        }
    }

    /// A peer will not deliver this chunk after all, so another may be asked.
    pub fn release(&mut self, chunk: Chunk) {
        if let Some(state) = self
            .partial
            .get_mut(&chunk.piece)
            .and_then(|partial| partial.chunks.get_mut(chunk.index as usize))
        {
            *state = match *state {
                ChunkState::Requested(copies) if copies > 1 => ChunkState::Requested(copies - 1),
                ChunkState::Requested(_) => ChunkState::Missing,
                other => other,
            };
        }
    }

    /// Whether data for this chunk would still be useful. Late data from a peer that was
    /// given up on is welcome, as long as nobody else has delivered it yet.
    pub fn wants_chunk(&self, chunk: Chunk) -> bool {
        if !self.is_needed(chunk.piece) {
            return false;
        }
        match self.partial.get(&chunk.piece) {
            Some(partial) => partial
                .chunks
                .get(chunk.index as usize)
                .is_some_and(|state| *state != ChunkState::Written),
            None => chunk.index < self.chunks_in(chunk.piece),
        }
    }

    /// Records a chunk that has reached the disk.
    pub fn chunk_written(&mut self, chunk: Chunk, peer: u64) -> Written {
        if !self.is_needed(chunk.piece) {
            return Written::Duplicate;
        }
        let chunks = self.chunks_in(chunk.piece);
        if chunk.index >= chunks {
            return Written::Duplicate;
        }
        let partial = self
            .partial
            .entry(chunk.piece)
            .or_insert_with(|| PartialPiece::new(chunks));
        let state = &mut partial.chunks[chunk.index as usize];
        if *state == ChunkState::Written {
            return Written::Duplicate;
        }
        *state = ChunkState::Written;
        partial.written += 1;
        partial.contributors.insert(peer);
        if partial.written < chunks {
            return Written::Stored;
        }
        self.verifying.insert(chunk.piece);
        Written::PieceComplete
    }

    /// How many peers are currently asked for this chunk.
    pub fn copies_out(&self, chunk: Chunk) -> u8 {
        match self
            .partial
            .get(&chunk.piece)
            .and_then(|partial| partial.chunks.get(chunk.index as usize))
        {
            Some(ChunkState::Requested(copies)) => *copies,
            _ => 0,
        }
    }

    /// Records a good hash. Returns true when this finished the piece.
    pub fn piece_passed(&mut self, piece: u32) -> bool {
        self.verifying.remove(&piece);
        self.partial.remove(&piece);
        self.order_dirty = true;
        let added = self.have.set(piece);
        if added && self.wanted.get(piece) {
            self.remaining -= 1;
        }
        added
    }

    /// Records a bad hash and returns the peers that sent the piece, for blame. All of
    /// it must be fetched again, because the bad chunk can't be told from the good ones.
    pub fn piece_failed(&mut self, piece: u32) -> Vec<u64> {
        self.verifying.remove(&piece);
        self.order_dirty = true;
        self.order_built = None;
        self.partial
            .remove(&piece)
            .map(|partial| partial.contributors.into_iter().collect())
            .unwrap_or_default()
    }

    /// Whether some of this piece has arrived or is being checked, but it isn't ours yet.
    /// A piece waiting for its hash check stays in `partial` until the check ends.
    pub fn is_in_progress(&self, piece: u32) -> bool {
        self.partial.contains_key(&piece)
    }

    /// Wanted pieces started but not finished, including ones waiting for their hash check.
    /// Skipped files keep their written chunks, and those don't count.
    pub fn in_progress_count(&self) -> usize {
        self.partial
            .keys()
            .filter(|piece| self.wanted.get(**piece))
            .count()
    }

    /// Forgets every finished and half-finished piece, so that only what passes a new check
    /// of the files counts. Only for a torrent whose swarm has stopped.
    pub fn forget_progress(&mut self) {
        self.have = Bitfield::new(self.lengths.total_pieces());
        self.partial.clear();
        self.verifying.clear();
        self.remaining = count_remaining(&self.have, &self.wanted);
        self.order_dirty = true;
        self.order_built = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librqbit_core::constants::CHUNK_SIZE;

    const PIECE: u32 = CHUNK_SIZE * 4;

    fn picker_for(pieces: u32) -> Picker {
        let lengths = Lengths::new(u64::from(pieces) * u64::from(PIECE), PIECE).unwrap();
        Picker::new(
            lengths,
            Bitfield::new(pieces),
            Bitfield::full(pieces),
            0x1234,
        )
    }

    fn holding(pieces: u32, held: &[u32]) -> Bitfield {
        let mut field = Bitfield::new(pieces);
        for piece in held {
            field.set(*piece);
        }
        field
    }

    fn nothing_in_flight(_: Chunk) -> bool {
        false
    }

    fn next(picker: &mut Picker, has: &Bitfield, limit: usize) -> Vec<ChunkRequest> {
        // A clock that moves on each call, so the order is rebuilt every time.
        let now = Instant::now() + picker.order_built.map_or(Duration::ZERO, |_| ORDER_REFRESH);
        picker.order_built = None;
        picker.next_requests(has, &nothing_in_flight, limit, now)
    }

    fn write_all_chunks(picker: &mut Picker, piece: u32, peer: u64) -> Written {
        let mut last = Written::Duplicate;
        for index in 0..picker.chunks_in(piece) {
            last = picker.chunk_written(Chunk { piece, index }, peer);
        }
        last
    }

    #[test]
    fn test_rarest_piece_goes_first() {
        let mut picker = picker_for(3);
        // Pieces 0 and 1 are on three peers, and piece 2 on one.
        for _ in 0..2 {
            picker.peer_added(&holding(3, &[0, 1]));
        }
        let everything = holding(3, &[0, 1, 2]);
        picker.peer_added(&everything);
        let requests = next(&mut picker, &everything, 1);
        assert_eq!(requests[0].chunk.piece, 2);
    }

    #[test]
    fn test_equally_rare_pieces_are_picked_in_a_random_order() {
        let everything = Bitfield::full(200);
        let firsts: HashSet<u32> = (0..20)
            .map(|seed| {
                let lengths = Lengths::new(200 * u64::from(PIECE), PIECE).unwrap();
                let mut picker = Picker::new(
                    lengths,
                    Bitfield::new(200),
                    Bitfield::full(200),
                    0x9E37_79B9_7F4A_7C15_u64.wrapping_mul(seed + 1),
                );
                next(&mut picker, &everything, 1)[0].chunk.piece
            })
            .collect();
        assert!(firsts.len() > 5, "{firsts:?}");
    }

    #[test]
    fn test_a_peer_is_only_asked_for_what_it_has() {
        let mut picker = picker_for(4);
        let has = holding(4, &[2]);
        picker.peer_added(&has);
        let requests = next(&mut picker, &has, 100);
        assert_eq!(requests.len(), 4);
        assert!(requests.iter().all(|r| r.chunk.piece == 2));
    }

    #[test]
    fn test_a_started_piece_is_finished_before_a_new_one_starts() {
        let mut picker = picker_for(3);
        let everything = Bitfield::full(3);
        picker.peer_added(&everything);
        let first = next(&mut picker, &everything, 2);
        let piece = first[0].chunk.piece;
        // Two chunks are out, so the next two must come from the same piece.
        let second = next(&mut picker, &everything, 2);
        assert!(second.iter().all(|r| r.chunk.piece == piece), "{second:?}");
    }

    #[test]
    fn test_requests_carry_the_right_bytes() {
        let lengths = Lengths::new(u64::from(CHUNK_SIZE) + 100, CHUNK_SIZE * 2).unwrap();
        let mut picker = Picker::new(lengths, Bitfield::new(1), Bitfield::full(1), 7);
        let everything = Bitfield::full(1);
        let requests = next(&mut picker, &everything, 10);
        assert_eq!(requests.len(), 2);
        assert_eq!((requests[0].begin, requests[0].length), (0, CHUNK_SIZE));
        assert_eq!((requests[1].begin, requests[1].length), (CHUNK_SIZE, 100));
    }

    #[test]
    fn test_pieces_not_selected_are_never_requested() {
        let lengths = Lengths::new(4 * u64::from(PIECE), PIECE).unwrap();
        let wanted = holding(4, &[1]);
        let mut picker = Picker::new(lengths, Bitfield::new(4), wanted, 7);
        let everything = Bitfield::full(4);
        let requests = next(&mut picker, &everything, 100);
        assert!(requests.iter().all(|r| r.chunk.piece == 1));
    }

    #[test]
    fn test_pieces_we_have_are_never_requested() {
        let lengths = Lengths::new(2 * u64::from(PIECE), PIECE).unwrap();
        let mut picker = Picker::new(lengths, holding(2, &[0]), Bitfield::full(2), 7);
        let everything = Bitfield::full(2);
        let requests = next(&mut picker, &everything, 100);
        assert!(requests.iter().all(|r| r.chunk.piece == 1));
    }

    #[test]
    fn test_a_released_chunk_can_be_requested_again() {
        let mut picker = picker_for(1);
        let everything = Bitfield::full(1);
        let first = next(&mut picker, &everything, 1);
        picker.release(first[0].chunk);
        let again = next(&mut picker, &everything, 1);
        assert_eq!(again[0], first[0]);
    }

    #[test]
    fn test_endgame_asks_a_second_peer_for_what_is_still_out() {
        let mut picker = picker_for(1);
        let everything = Bitfield::full(1);
        assert_eq!(next(&mut picker, &everything, 100).len(), 4);
        let again = next(&mut picker, &everything, 100);
        assert_eq!(again.len(), 4);
    }

    #[test]
    fn test_endgame_skips_what_this_peer_was_already_asked() {
        let mut picker = picker_for(1);
        let everything = Bitfield::full(1);
        let first = next(&mut picker, &everything, 100);
        let same_peer = |_: Chunk| true;
        let requests = picker.next_requests(&everything, &same_peer, 100, Instant::now());
        assert!(requests.is_empty(), "{first:?} then {requests:?}");
    }

    #[test]
    fn test_endgame_has_a_limit_on_copies() {
        let mut picker = picker_for(1);
        let everything = Bitfield::full(1);
        for _ in 0..u32::from(MAX_ENDGAME_COPIES) {
            assert_eq!(next(&mut picker, &everything, 100).len(), 4);
        }
        assert!(next(&mut picker, &everything, 100).is_empty());
    }

    #[test]
    fn test_a_chunk_written_twice_counts_once() {
        let mut picker = picker_for(1);
        let chunk = Chunk { piece: 0, index: 0 };
        assert_eq!(picker.chunk_written(chunk, 1), Written::Stored);
        assert_eq!(picker.chunk_written(chunk, 2), Written::Duplicate);
        assert!(!picker.wants_chunk(chunk));
    }

    #[test]
    fn test_the_last_chunk_completes_the_piece() {
        let mut picker = picker_for(1);
        assert_eq!(write_all_chunks(&mut picker, 0, 1), Written::PieceComplete);
        assert!(!picker.have().get(0), "not ours until the hash passes");
        assert!(picker.piece_passed(0));
        assert!(picker.have().get(0));
        assert!(picker.is_finished());
    }

    #[test]
    fn test_a_piece_being_hashed_is_not_requested() {
        let mut picker = picker_for(1);
        let everything = Bitfield::full(1);
        write_all_chunks(&mut picker, 0, 1);
        assert!(next(&mut picker, &everything, 100).is_empty());
    }

    #[test]
    fn test_a_failed_piece_is_fetched_again_and_names_its_senders() {
        let mut picker = picker_for(1);
        let everything = Bitfield::full(1);
        picker.chunk_written(Chunk { piece: 0, index: 0 }, 5);
        picker.chunk_written(Chunk { piece: 0, index: 1 }, 6);
        picker.chunk_written(Chunk { piece: 0, index: 2 }, 5);
        picker.chunk_written(Chunk { piece: 0, index: 3 }, 6);
        let mut blamed = picker.piece_failed(0);
        blamed.sort_unstable();
        assert_eq!(blamed, [5, 6]);
        assert_eq!(next(&mut picker, &everything, 100).len(), 4);
    }

    #[test]
    fn test_only_wanted_pieces_decide_when_it_is_finished() {
        let lengths = Lengths::new(2 * u64::from(PIECE), PIECE).unwrap();
        let mut picker = Picker::new(lengths, Bitfield::new(2), holding(2, &[1]), 7);
        assert!(!picker.is_finished());
        write_all_chunks(&mut picker, 1, 1);
        picker.piece_passed(1);
        assert!(picker.is_finished());
    }

    #[test]
    fn test_late_data_is_still_useful_after_release() {
        let mut picker = picker_for(1);
        let everything = Bitfield::full(1);
        let request = next(&mut picker, &everything, 1)[0];
        picker.release(request.chunk);
        assert!(picker.wants_chunk(request.chunk));
        assert_eq!(picker.chunk_written(request.chunk, 1), Written::Stored);
    }

    #[test]
    fn test_chunks_outside_the_piece_are_refused() {
        let mut picker = picker_for(1);
        let past = Chunk { piece: 0, index: 4 };
        assert!(!picker.wants_chunk(past));
        assert_eq!(picker.chunk_written(past, 1), Written::Duplicate);
        let other = Chunk { piece: 9, index: 0 };
        assert!(!picker.wants_chunk(other));
        assert_eq!(picker.chunk_written(other, 1), Written::Duplicate);
    }

    #[test]
    fn test_a_peer_leaving_lowers_availability() {
        let mut picker = picker_for(2);
        let one = holding(2, &[0]);
        let everything = Bitfield::full(2);
        picker.peer_added(&one);
        picker.peer_added(&everything);
        picker.peer_removed(&one);
        // Now both pieces have one holder, and leaving twice must not underflow.
        picker.peer_removed(&one);
        picker.peer_removed(&one);
        assert_eq!(picker.availability, [0, 1]);
    }

    #[test]
    fn test_interest_follows_what_is_missing() {
        let mut picker = picker_for(1);
        let everything = Bitfield::full(1);
        assert!(picker.is_interesting(&everything));
        write_all_chunks(&mut picker, 0, 1);
        picker.piece_passed(0);
        assert!(!picker.is_interesting(&everything));
    }

    #[test]
    fn test_selecting_a_piece_later_keeps_written_chunks() {
        let mut picker = picker_for(1);
        picker.chunk_written(Chunk { piece: 0, index: 0 }, 1);
        picker.set_wanted(Bitfield::new(1));
        picker.set_wanted(Bitfield::full(1));
        assert!(!picker.wants_chunk(Chunk { piece: 0, index: 0 }));
    }
}
