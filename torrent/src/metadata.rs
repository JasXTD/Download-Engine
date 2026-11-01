//! Fetches a torrent's details from a peer, which is how a magnet link becomes a torrent.
//! Only the hash is known at the start, and the peer is a stranger, so its claims are limited
//! here and the result counts for nothing until it matches the hash.

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use buffers::ByteBuf;
use bytes::{Bytes, BytesMut};
use librqbit_core::constants::CHUNK_SIZE;
use librqbit_core::hash_id::Id20;
use librqbit_peer_protocol::extended::handshake::ExtendedHandshake;
use librqbit_peer_protocol::extended::ut_metadata::{UtMetadata, UtMetadataData};
use librqbit_peer_protocol::extended::{ExtendedMessage, PeerExtendedMessageIds};
use librqbit_peer_protocol::{MAX_MSG_LEN, Message, MessageDeserializeError};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::peer::{BoxedStream, CLIENT_NAME, frame_len, read_handshake, send_handshake};

/// Real torrents need far less. This stops a peer from claiming a size that fills the memory.
pub(crate) const MAX_METADATA_SIZE: u32 = 32 * 1024 * 1024;
const REQUEST_WINDOW: u32 = 8;
/// A bitfield can arrive first, and it can be larger than other messages.
const MAX_FRAME: usize = 1 << 20;

/// Collects the pieces of the info dictionary.
pub(crate) struct Collector {
    buffer: Vec<u8>,
    received: Vec<bool>,
    missing: usize,
}

impl Collector {
    pub fn new(size: u32) -> Result<Self> {
        ensure!(
            size > 0 && size <= MAX_METADATA_SIZE,
            "peer claims the torrent details are {size} bytes"
        );
        let pieces = size.div_ceil(CHUNK_SIZE) as usize;
        Ok(Self {
            buffer: vec![0; size as usize],
            received: vec![false; pieces],
            missing: pieces,
        })
    }

    pub fn pieces(&self) -> u32 {
        self.received.len() as u32
    }

    /// Stores a piece. Returns true when all of them are here.
    pub fn store(&mut self, data: &UtMetadataData<ByteBuf<'_>>) -> Result<bool> {
        let piece = data.piece() as usize;
        let start = piece * CHUNK_SIZE as usize;
        let end = (start + CHUNK_SIZE as usize).min(self.buffer.len());
        ensure!(
            piece < self.received.len() && data.len() == end - start,
            "peer sent a piece of the torrent details that doesn't fit"
        );
        data.copy_to_slice(&mut self.buffer[start..end]);
        if !self.received[piece] {
            self.received[piece] = true;
            self.missing -= 1;
        }
        Ok(self.missing == 0)
    }

    pub fn finish(self) -> Bytes {
        self.buffer.into()
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
pub(crate) async fn fetch(mut stream: BoxedStream, info_hash: Id20, our_id: Id20) -> Result<Bytes> {
    send_handshake(&mut stream, info_hash, our_id).await?;
    let (their_hash, hello) = read_handshake(&mut stream).await?;
    ensure!(their_hash == info_hash, "peer is on another torrent");
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
                    ids = theirs.peer_extended_messages();
                    ensure!(ids.ut_metadata.is_some(), "peer can't send torrent details");
                    let size = theirs
                        .metadata_size
                        .context("peer didn't say how big the torrent details are")?;
                    let new = Collector::new(size)?;
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
                        return collector
                            .take()
                            .map(Collector::finish)
                            .context("the torrent details went missing");
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
        assert!(Collector::new(0).is_err());
        assert!(Collector::new(MAX_METADATA_SIZE + 1).is_err());
        assert!(Collector::new(u32::MAX).is_err());
        assert!(Collector::new(MAX_METADATA_SIZE).is_ok());
    }

    #[test]
    fn test_pieces_can_arrive_in_any_order_and_twice() {
        let size = CHUNK_SIZE + 10;
        let mut collector = Collector::new(size).unwrap();
        let first = vec![1u8; CHUNK_SIZE as usize];
        let second = vec![2u8; 10];
        let late = UtMetadataData::from_bytes(1, size, ByteBuf(&second));
        assert!(!collector.store(&late).unwrap());
        assert!(!collector.store(&late).unwrap());
        let early = UtMetadataData::from_bytes(0, size, ByteBuf(&first));
        assert!(collector.store(&early).unwrap());
        let bytes = collector.finish();
        assert_eq!(bytes[0], 1);
        assert_eq!(bytes[CHUNK_SIZE as usize], 2);
    }

    #[test]
    fn test_a_piece_of_the_wrong_size_is_refused() {
        let mut collector = Collector::new(100).unwrap();
        let short = vec![1u8; 50];
        let data = UtMetadataData::from_bytes(0, 100, ByteBuf(&short));
        assert!(collector.store(&data).is_err());
    }

    #[test]
    fn test_a_piece_past_the_end_is_refused() {
        let mut collector = Collector::new(100).unwrap();
        let data = vec![1u8; 100];
        let wrong = UtMetadataData::from_bytes(1, 100, ByteBuf(&data));
        assert!(collector.store(&wrong).is_err());
    }
}
