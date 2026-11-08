//!  Fetches a torrent's details from a peer, which is how a magnet link becomes a torrent.
//! Only the hash is known at the start, and the peer is a stranger, so its claims are limited
//! here and the result counts for nothing until it matches the hash.

#[cfg(test)]
use crate::peer::send_handshake;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use buffers::ByteBuf;
use bytes::{Bytes, BytesMut};
use librqbit_core::constants::CHUNK_SIZE;
use librqbit_core::hash_id::Id20;
use librqbit_peer_protocol::extended::handshake::ExtendedHandshake;
use librqbit_peer_protocol::extended::ut_metadata::{UtMetadata, UtMetadataData};
use librqbit_peer_protocol::extended::{ExtendedMessage, PeerExtendedMessageIds};
use librqbit_peer_protocol::{MAX_MSG_LEN, Message, MessageDeserializeError};
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::peer::{BoxedStream, CLIENT_NAME, frame_len, read_handshake};

/// Real torrents need far less. This stops a peer from claiming a size that fills the memory.
pub(crate) const MAX_METADATA_SIZE: u32 = 32 * 1024 * 1024;
const REQUEST_WINDOW: u32 = 8;
/// A bitfield can arrive first, and it can be larger than other messages.
const MAX_FRAME: usize = 1 << 20;
const TORRENT_METADATA_BUDGET: usize = 64 * 1024 * 1024;
const GLOBAL_METADATA_BUDGET: usize = 128 * 1024 * 1024;
static METADATA_BUDGET: OnceLock<Arc<Semaphore>> = OnceLock::new();

/// Reservations cover both lazy chunks and the final contiguous copy, and survive
/// until the caller has validated the result. Dropping a cancelled attempt releases them.
#[derive(Clone)]
pub(crate) struct Budget {
    torrent: Arc<Semaphore>,
    global: Arc<Semaphore>,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            torrent: Arc::new(Semaphore::new(TORRENT_METADATA_BUDGET)),
            global: METADATA_BUDGET
                .get_or_init(|| Arc::new(Semaphore::new(GLOBAL_METADATA_BUDGET)))
                .clone(),
        }
    }
}

struct Reservation {
    _torrent: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

impl Budget {
    fn reserve(&self, size: u32) -> Result<Reservation> {
        let bytes = size.checked_mul(2).context("metadata size overflow")?;
        let torrent = self
            .torrent
            .clone()
            .try_acquire_many_owned(bytes)
            .context("torrent metadata memory budget is full")?;
        let global = self
            .global
            .clone()
            .try_acquire_many_owned(bytes)
            .context("global metadata memory budget is full")?;
        Ok(Reservation {
            _torrent: torrent,
            _global: global,
        })
    }
}

pub(crate) struct Fetched {
    pub bytes: Bytes,
    pub layers: Option<Vec<u8>>,
    _reservation: Reservation,
    _layer_reservation: Option<Reservation>,
}

/// Collects the pieces of the info dictionary.
pub(crate) struct Collector {
    size: usize,
    chunks: Vec<Option<Vec<u8>>>,
    missing: usize,
    reservation: Reservation,
}

impl Collector {
    pub fn new(size: u32, budget: &Budget) -> Result<Self> {
        ensure!(
            size > 0 && size <= MAX_METADATA_SIZE,
            "peer claims the torrent details are {size} bytes"
        );
        let pieces = size.div_ceil(CHUNK_SIZE) as usize;
        let reservation = budget.reserve(size)?;
        Ok(Self {
            size: size as usize,
            chunks: vec![None; pieces],
            missing: pieces,
            reservation,
        })
    }

    pub fn pieces(&self) -> u32 {
        self.chunks.len() as u32
    }

    /// Stores a piece. Returns true when all of them are here.
    pub fn store(&mut self, data: &UtMetadataData<ByteBuf<'_>>) -> Result<bool> {
        let piece = data.piece() as usize;
        ensure!(
            piece < self.chunks.len(),
            "peer sent a metadata piece past the end"
        );
        let start = piece * CHUNK_SIZE as usize;
        let end = (start + CHUNK_SIZE as usize).min(self.size);
        ensure!(
            data.len() == end - start,
            "peer sent a piece of the torrent details that doesn't fit"
        );
        if self.chunks[piece].is_none() {
            let mut bytes = vec![0; data.len()];
            data.copy_to_slice(&mut bytes);
            self.chunks[piece] = Some(bytes);
            self.missing -= 1;
        }
        Ok(self.missing == 0)
    }

    pub fn finish(self) -> Result<Fetched> {
        ensure!(self.missing == 0, "torrent details are incomplete");
        let mut bytes = Vec::with_capacity(self.size);
        for chunk in self.chunks {
            bytes.extend_from_slice(&chunk.context("missing metadata chunk")?);
        }
        Ok(Fetched {
            bytes: bytes.into(),
            layers: None,
            _reservation: self.reservation,
            _layer_reservation: None,
        })
    }
}

async fn send(
    writer: &mut (impl AsyncWrite + Unpin),
    message: Message<'_>,
    ids: PeerExtendedMessageIds,
) -> Result<()> {
    let mut buffer = vec![0u8; MAX_MSG_LEN];
    let len = message
        .serialize(&mut buffer, &|| ids)
        .map_err(|error| anyhow!("{error}"))?;
    writer.write_all(&buffer[..len]).await?;
    Ok(())
}

async fn request(
    writer: &mut (impl AsyncWrite + Unpin),
    piece: u32,
    ids: PeerExtendedMessageIds,
) -> Result<()> {
    let message = Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(piece)));
    send(writer, message, ids).await
}

/// Asks one peer for the torrent's details. The caller limits how long this may take, and
/// must check the result against the info hash.
#[cfg(test)]
pub(crate) async fn fetch(
    stream: BoxedStream,
    hashes: impl Into<crate::v2::InfoHashes>,
    our_id: Id20,
    budget: Budget,
) -> Result<Fetched> {
    let hashes = hashes.into();
    let info_hash = hashes.wire();
    fetch_on(stream, hashes, info_hash, our_id, budget).await
}

pub(crate) async fn fetch_on(
    mut stream: BoxedStream,
    hashes: crate::v2::InfoHashes,
    info_hash: Id20,
    our_id: Id20,
    budget: Budget,
) -> Result<Fetched> {
    crate::peer::send_v2_handshake(&mut stream, info_hash, our_id, hashes.v2.is_some()).await?;
    let (their_hash, hello) = read_handshake(&mut stream).await?;
    ensure!(hashes.contains(their_hash), "peer is on another torrent");
    ensure!(hello.peer_id != our_id, "connected to ourselves");
    ensure!(hello.supports_extended, "peer can't send torrent details");

    let mut handshake = ExtendedHandshake::new();
    handshake.v = Some(ByteBuf(CLIENT_NAME.as_bytes()));
    handshake.m.ut_pex = None;
    let mut ids = PeerExtendedMessageIds::default();
    send(
        &mut stream,
        Message::Extended(ExtendedMessage::Handshake(handshake)),
        ids,
    )
    .await?;

    let mut collector: Option<Collector> = None;
    let mut next_piece = 0;
    let mut buffer = BytesMut::with_capacity(32 * 1024);
    loop {
        while let Some(len) = frame_len(&buffer, MAX_FRAME)? {
            let frame = buffer.split_to(len).freeze();
            let message = match Message::deserialize(&frame, &[]) {
                Ok((message, _)) => message,
                Err(MessageDeserializeError::UnsupportedMessageId(_)) => continue,
                Err(error) => bail!("bad message: {error}"),
            };
            let Message::Extended(extended) = message else {
                continue;
            };
            match extended {
                ExtendedMessage::Handshake(theirs) => {
                    ensure!(collector.is_none(), "peer repeated its metadata handshake");
                    ids = theirs.peer_extended_messages();
                    ensure!(ids.ut_metadata.is_some(), "peer can't send torrent details");
                    let size = theirs
                        .metadata_size
                        .context("peer didn't say how big the torrent details are")?;
                    let new = Collector::new(size, &budget)?;
                    while next_piece < REQUEST_WINDOW.min(new.pieces()) {
                        request(&mut stream, next_piece, ids).await?;
                        next_piece += 1;
                    }
                    collector = Some(new);
                }
                ExtendedMessage::UtMetadata(UtMetadata::Data(data)) => {
                    let current = collector
                        .as_mut()
                        .context("peer sent torrent details before its handshake")?;
                    if current.store(&data)? {
                        let mut fetched = collector
                            .take()
                            .context("the torrent details went missing")?
                            .finish()?;
                        let mut meta = crate::meta::TorrentMeta::from_hashes(
                            hashes,
                            fetched.bytes.clone(),
                            Vec::new(),
                        )?;
                        if meta.hashes.v2.is_some() {
                            let layer_bytes = meta
                                .v2
                                .iter()
                                .filter(|f| f.piece_count > 1)
                                .try_fold(0u32, |n, f| {
                                    n.checked_add(f.piece_count.checked_mul(32)?)
                                })
                                .context("V2 layer size overflow")?;
                            fetched._layer_reservation = Some(budget.reserve(layer_bytes)?);
                            crate::v2_wire::fetch_layers(&mut stream, &mut buffer, &mut meta)
                                .await?;
                            fetched.layers = Some(crate::v2::encode_layers(&meta)?);
                        }
                        return Ok(fetched);
                    }
                    if next_piece < current.pieces() {
                        request(&mut stream, next_piece, ids).await?;
                        next_piece += 1;
                    }
                }
                ExtendedMessage::UtMetadata(UtMetadata::Reject(_)) => {
                    bail!("peer won't send the torrent details");
                }
                _ => {}
            }
        }
        buffer.reserve(32 * 1024);
        if stream.read_buf(&mut buffer).await? == 0 {
            bail!("peer closed the connection");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_impossible_sizes_are_refused_before_any_memory_is_used() {
        let budget = test_budget(TORRENT_METADATA_BUDGET, GLOBAL_METADATA_BUDGET);
        assert!(Collector::new(0, &budget).is_err());
        assert!(Collector::new(MAX_METADATA_SIZE + 1, &budget).is_err());
        assert!(Collector::new(u32::MAX, &budget).is_err());
        assert!(Collector::new(MAX_METADATA_SIZE, &budget).is_ok());
    }

    #[test]
    fn test_pieces_can_arrive_in_any_order_and_twice() {
        let size = CHUNK_SIZE + 10;
        let mut collector = Collector::new(size, &Budget::default()).unwrap();
        let first = vec![1u8; CHUNK_SIZE as usize];
        let second = vec![2u8; 10];
        let late = UtMetadataData::from_bytes(1, size, ByteBuf(&second));
        assert!(!collector.store(&late).unwrap());
        assert!(!collector.store(&late).unwrap());
        let early = UtMetadataData::from_bytes(0, size, ByteBuf(&first));
        assert!(collector.store(&early).unwrap());
        let bytes = collector.finish().unwrap().bytes;
        assert_eq!(bytes[0], 1);
        assert_eq!(bytes[CHUNK_SIZE as usize], 2);
    }

    #[test]
    fn test_a_piece_of_the_wrong_size_is_refused() {
        let mut collector = Collector::new(100, &Budget::default()).unwrap();
        let short = vec![1u8; 50];
        let data = UtMetadataData::from_bytes(0, 100, ByteBuf(&short));
        assert!(collector.store(&data).is_err());
    }

    #[test]
    fn test_a_piece_past_the_end_is_refused() {
        let mut collector = Collector::new(100, &Budget::default()).unwrap();
        let data = vec![1u8; 100];
        let wrong = UtMetadataData::from_bytes(1, 100, ByteBuf(&data));
        assert!(collector.store(&wrong).is_err());
    }

    fn test_budget(torrent: usize, global: usize) -> Budget {
        Budget {
            torrent: Arc::new(Semaphore::new(torrent)),
            global: Arc::new(Semaphore::new(global)),
        }
    }

    #[test]
    fn metadata_is_lazy_and_budgets_cover_completed_results() {
        let budget = test_budget(200, 200);
        let mut collector = Collector::new(100, &budget).unwrap();
        assert!(collector.chunks.iter().all(Option::is_none));
        assert!(Collector::new(1, &budget).is_err());
        collector
            .store(&UtMetadataData::from_bytes(0, 100, ByteBuf(&[7; 100])))
            .unwrap();
        let complete = collector.finish().unwrap();
        assert_eq!(complete.bytes.as_ref(), &[7; 100]);
        assert!(Collector::new(1, &budget).is_err());
        drop(complete);
        assert_eq!(budget.torrent.available_permits(), 200);
        assert_eq!(budget.global.available_permits(), 200);
    }

    #[test]
    fn global_budget_is_shared_across_torrents_and_failure_releases_local_reservation() {
        let one = test_budget(200, 200);
        let two = Budget {
            torrent: Arc::new(Semaphore::new(200)),
            global: one.global.clone(),
        };
        let collector = Collector::new(100, &one).unwrap();
        assert!(Collector::new(100, &two).is_err());
        assert_eq!(two.torrent.available_permits(), 200);
        drop(collector);
        assert!(Collector::new(100, &two).is_ok());
    }

    #[tokio::test]
    async fn repeated_metadata_handshake_is_rejected_and_releases_budget() {
        let hash = Id20::new([1; 20]);
        let (client, mut peer) = tokio::io::duplex(8192);
        let budget = test_budget(200, 200);
        let exchange = async {
            read_handshake(&mut peer).await.unwrap();
            send_handshake(&mut peer, hash, Id20::new([3; 20]))
                .await
                .unwrap();
            for _ in 0..2 {
                let mut hello = ExtendedHandshake::new();
                hello.metadata_size = Some(100);
                send(
                    &mut peer,
                    Message::Extended(ExtendedMessage::Handshake(hello)),
                    PeerExtendedMessageIds::default(),
                )
                .await
                .unwrap();
            }
            std::future::pending::<()>().await;
        };
        let result = tokio::select! {
            result = fetch(Box::new(client), hash, Id20::new([2; 20]), budget.clone()) => result,
            _ = exchange => unreachable!(),
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => panic!("handshake rejection stalled"),
        };
        assert!(result.err().unwrap().to_string().contains("repeated"));
        assert_eq!(budget.global.available_permits(), 200);
    }

    #[tokio::test]
    async fn aborting_a_metadata_fetch_releases_all_reservations() {
        let hash = Id20::new([1; 20]);
        let (client, mut peer) = tokio::io::duplex(8192);
        let budget = test_budget(200, 200);
        let attempt = tokio::spawn(fetch(
            Box::new(client),
            hash,
            Id20::new([2; 20]),
            budget.clone(),
        ));
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            read_handshake(&mut peer).await.unwrap();
            send_handshake(&mut peer, hash, Id20::new([3; 20]))
                .await
                .unwrap();
            let mut hello = ExtendedHandshake::new();
            hello.metadata_size = Some(100);
            send(
                &mut peer,
                Message::Extended(ExtendedMessage::Handshake(hello)),
                PeerExtendedMessageIds::default(),
            )
            .await
            .unwrap();
            loop {
                let len = peer.read_u32().await.unwrap() as usize;
                let mut frame = vec![0; len + 4];
                frame[..4].copy_from_slice(&(len as u32).to_be_bytes());
                peer.read_exact(&mut frame[4..]).await.unwrap();
                if matches!(
                    Message::deserialize(&frame, &[]).unwrap().0,
                    Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Request(0)))
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(budget.global.available_permits(), 0);
        attempt.abort();
        assert!(attempt.await.err().unwrap().is_cancelled());
        assert_eq!(budget.global.available_permits(), 200);
        assert_eq!(budget.torrent.available_permits(), 200);
    }
}
