//! One connection to one peer. Everything a peer sends comes from a stranger, so every
//! number is checked before use, and every queue has a limit.
//!
//! Reading and writing run side by side. If they took turns, two peers each busy sending
//! to the other could wait on each other forever. Other parts of the engine reach a peer
//! through its `PeerLink`, and only ever add to a queue, so nothing can get stuck on a slow
//! peer.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use buffers::ByteBuf;
use bytes::{Bytes, BytesMut};
use librqbit_core::constants::CHUNK_SIZE;
use librqbit_core::hash_id::Id20;
use librqbit_peer_protocol::extended::handshake::ExtendedHandshake;
use librqbit_peer_protocol::extended::ut_metadata::{UtMetadata, UtMetadataData};
use librqbit_peer_protocol::extended::{ExtendedMessage, PeerExtendedMessageIds};
use librqbit_peer_protocol::{Handshake, MAX_MSG_LEN, Message, MessageDeserializeError, Request};
use parking_lot::Mutex;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::Notify;
use tokio::time::{Instant, MissedTickBehavior, timeout};
use tokio_util::sync::CancellationToken;

use crate::bitfield::Bitfield;
use crate::debug_log;
use crate::details::{MAX_CLIENT_CHARS, PeerDetails, clean_text};
use crate::picker::{Chunk, Written};
use crate::torrent::{Event, Live, Rates};

const HANDSHAKE_LEN: usize = 68;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);
const KEEP_ALIVE: Duration = Duration::from_secs(90);
/// Peers should send something at least every two minutes.
const IDLE_TIMEOUT: Duration = Duration::from_secs(180);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// A peer that lets requests time out this many times in a row is dropped.
const MAX_TIMEOUTS: u32 = 3;
const TICK: Duration = Duration::from_secs(2);
const READ_BUFFER: usize = 64 * 1024;
/// Control messages are grouped into one write up to about this size.
const WRITE_BATCH: usize = 16 * 1024;
const PIECE_HEADER: usize = 13;
pub const CLIENT_NAME: &str = "Download Engine";

/// How many requests to keep open with a peer. It follows the peer's speed, so that a fast
/// peer never sits idle waiting for the next request.
const MIN_QUEUE: usize = 32;
const MAX_QUEUE: usize = 256;
/// How many blocks a peer may have waiting from us.
const MAX_UPLOAD_QUEUE: usize = 512;
const MAX_PENDING_METADATA: usize = 16;
/// Blocks nobody asked for could fill the disk, so a peer gets few chances.
const MAX_UNSOLICITED: u32 = 64;
const MAX_RELEASED: usize = 1024;
const MAX_PEX_PEERS: usize = 50;

/// TCP/uTP streams, optionally wrapped in MSE; in-memory pipes in Windows tests.
pub trait PeerStream: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> PeerStream for T {}
pub type BoxedStream = Box<dyn PeerStream>;

#[derive(Clone, Copy, Debug)]
pub struct Hello {
    pub connection: crate::details::ConnectionInfo,
    pub peer_id: Id20,
    pub supports_extended: bool,
}

pub async fn send_handshake(
    stream: &mut (impl AsyncWrite + Unpin),
    info_hash: Id20,
    our_id: Id20,
) -> Result<()> {
    let mut buffer = [0u8; HANDSHAKE_LEN];
    let len = Handshake::new(info_hash, our_id).serialize_unchecked_len(&mut buffer);
    timeout(HANDSHAKE_TIMEOUT, stream.write_all(&buffer[..len]))
        .await
        .context("timed out sending the handshake")??;
    Ok(())
}

/// Reads a peer's handshake, and returns which torrent it is for.
pub async fn read_handshake(stream: &mut (impl AsyncRead + Unpin)) -> Result<(Id20, Hello)> {
    let mut buffer = [0u8; HANDSHAKE_LEN];
    timeout(HANDSHAKE_TIMEOUT, stream.read_exact(&mut buffer))
        .await
        .context("timed out waiting for the handshake")??;
    let (handshake, _) = Handshake::deserialize(&buffer).map_err(|error| anyhow!("{error}"))?;
    let hello = Hello {
        connection: Default::default(),
        peer_id: handshake.peer_id,
        supports_extended: handshake.supports_extended(),
    };
    Ok((handshake.info_hash, hello))
}

/// The length of the first message in `buffer`, once all of it has arrived. The size comes
/// from the peer, so it is checked before any memory is set aside for it.
pub(crate) fn frame_len(buffer: &[u8], max: usize) -> Result<Option<usize>> {
    let Some(prefix) = buffer.first_chunk::<4>() else {
        return Ok(None);
    };
    let total = u32::from_be_bytes(*prefix) as usize + 4;
    ensure!(total <= max, "peer announced a message of {total} bytes");
    Ok((buffer.len() >= total).then_some(total))
}

pub(crate) enum Out {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request(Request),
    Cancel(Request),
    ExtendedHandshake,
    MetadataPiece(u32),
    MetadataReject(u32),
}

#[derive(Default)]
struct Outbox {
    control: VecDeque<Out>,
    uploads: VecDeque<Request>,
    pending_metadata: usize,
}

/// What the rest of the engine can do with a connected peer.
pub(crate) struct PeerLink {
    pub connection: crate::details::ConnectionInfo,
    pub key: u64,
    pub peer_id: Id20,
    pub addr: Option<SocketAddr>,
    /// The peer connected to us.
    pub incoming: bool,
    outbox: Mutex<Outbox>,
    writer_wake: Notify,
    reader_wake: Notify,
    finished_chunks: Mutex<Vec<Chunk>>,
    extended_ids: Mutex<PeerExtendedMessageIds>,
    choking: AtomicBool,
    interested: AtomicBool,
    pub downloaded: AtomicU64,
    pub uploaded: AtomicU64,
    strikes: AtomicU32,
    kicked: CancellationToken,
    // What the status window shows. The reader keeps these up to date.
    client: Mutex<Option<String>>,
    has_pieces: AtomicU32,
    peer_choking: AtomicBool,
    am_interested: AtomicBool,
    rates: Mutex<Rates>,
}

impl PeerLink {
    pub fn new(key: u64, peer_id: Id20, addr: Option<SocketAddr>, incoming: bool) -> Self {
        Self {
            connection: Default::default(),
            key,
            peer_id,
            addr,
            incoming,
            outbox: Mutex::default(),
            writer_wake: Notify::new(),
            reader_wake: Notify::new(),
            finished_chunks: Mutex::default(),
            extended_ids: Mutex::default(),
            choking: AtomicBool::new(true),
            interested: AtomicBool::new(false),
            downloaded: AtomicU64::new(0),
            uploaded: AtomicU64::new(0),
            strikes: AtomicU32::new(0),
            kicked: CancellationToken::new(),
            client: Mutex::default(),
            has_pieces: AtomicU32::new(0),
            peer_choking: AtomicBool::new(true),
            am_interested: AtomicBool::new(false),
            rates: Mutex::default(),
        }
    }

    /// Takes this second's sample of the transfer speeds.
    pub fn sample_rates(&self) {
        self.rates.lock().sample(
            self.downloaded.load(Ordering::Relaxed),
            self.uploaded.load(Ordering::Relaxed),
        );
    }

    pub fn details(&self) -> PeerDetails {
        let (download_bps, upload_bps) = self.rates.lock().per_second();
        PeerDetails {
            connection: self.connection,
            address: self
                .addr
                .map(|addr| SocketAddr::new(addr.ip().to_canonical(), addr.port()).to_string()),
            client: self.client.lock().clone(),
            has_pieces: self.has_pieces.load(Ordering::Relaxed),
            download_bps,
            upload_bps,
            downloaded_bytes: self.downloaded.load(Ordering::Relaxed),
            uploaded_bytes: self.uploaded.load(Ordering::Relaxed),
            incoming: self.incoming,
            receiving: !self.peer_choking.load(Ordering::Relaxed)
                && self.am_interested.load(Ordering::Relaxed),
            sending: !self.is_choking() && self.is_interested(),
        }
    }

    pub fn push(&self, out: Out) {
        self.outbox.lock().control.push_back(out);
        self.writer_wake.notify_one();
    }

    /// Asks this peer's reader to look for work, because something changed.
    pub fn nudge(&self) {
        self.reader_wake.notify_one();
    }

    /// Another peer delivered this chunk, so asking this one is no longer needed.
    pub fn chunk_finished_elsewhere(&self, chunk: Chunk) {
        self.finished_chunks.lock().push(chunk);
        self.reader_wake.notify_one();
    }

    pub fn is_choking(&self) -> bool {
        self.choking.load(Ordering::Acquire)
    }

    pub fn is_interested(&self) -> bool {
        self.interested.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub fn mark_interested(&self) {
        self.interested.store(true, Ordering::Release);
    }

    pub fn choke(&self) {
        if !self.choking.swap(true, Ordering::AcqRel) {
            let mut outbox = self.outbox.lock();
            outbox.uploads.clear();
            outbox.control.push_back(Out::Choke);
            drop(outbox);
            self.writer_wake.notify_one();
        }
    }

    pub fn unchoke(&self) {
        if self.choking.swap(false, Ordering::AcqRel) {
            self.push(Out::Unchoke);
        }
    }

    /// Counts a bad piece against this peer. Returns true when it should be dropped.
    pub fn add_strike(&self, limit: u32) -> bool {
        self.strikes.fetch_add(1, Ordering::AcqRel) + 1 >= limit
    }

    pub fn kick(&self) {
        self.kicked.cancel();
    }

    fn queue_upload(&self, request: Request) {
        let mut outbox = self.outbox.lock();
        if outbox.uploads.len() < MAX_UPLOAD_QUEUE {
            outbox.uploads.push_back(request);
            drop(outbox);
            self.writer_wake.notify_one();
        }
    }

    fn cancel_upload(&self, request: Request) {
        self.outbox.lock().uploads.retain(|queued| {
            (queued.index, queued.begin, queued.length)
                != (request.index, request.begin, request.length)
        });
    }

    fn queue_metadata(&self, out: Out) {
        let mut outbox = self.outbox.lock();
        if outbox.pending_metadata < MAX_PENDING_METADATA {
            outbox.pending_metadata += 1;
            outbox.control.push_back(out);
            drop(outbox);
            self.writer_wake.notify_one();
        }
    }
}

/// Runs a connection after both handshakes, until it ends. Returns whether it was any use,
/// which decides how soon to try this peer again.
pub(crate) async fn run_connection(
    live: Arc<Live>,
    stream: BoxedStream,
    hello: Hello,
    addr: Option<SocketAddr>,
    incoming: bool,
) -> bool {
    let Some(link) = live.register_connection(hello.peer_id, addr, incoming, hello.connection)
    else {
        debug_log!("already connected to {:?}", addr);
        return false;
    };
    let (mut read_half, mut write_half) = tokio::io::split(stream);
    if hello.supports_extended {
        link.push(Out::ExtendedHandshake);
    }
    let have = live.core.picker.lock().have().clone();
    if have.count() > 0 {
        link.push(Out::Bitfield(have.as_bytes().to_vec()));
    }

    let mut reader = Reader::new(live.clone(), link.clone());
    let result = tokio::select! {
        biased;
        _ = live.cancel.cancelled() => Ok(()),
        _ = link.kicked.cancelled() => Err(anyhow!("sent bad data too often")),
        result = read_loop(&mut read_half, &mut reader) => result,
        result = write_loop(&mut write_half, &live, &link) => result,
    };
    // Cancelling read_loop drops its await, not a blocking disk operation. Join it
    // before releasing the connection so Pause/Stop all cannot return ahead of a write.
    if let Some(write) = reader.pending_write.take() {
        let error = match write.await {
            Ok((Ok(()), _lease)) => None,
            Ok((Err(error), _lease)) => {
                Some(format!("Could not write to the download folder: {error}"))
            }
            Err(error) => Some(format!("A peer disk write failed: {error}")),
        };
        if let Some(error) = error {
            let _ = live.events.send(Event::Fatal(error));
        }
    }
    if let Err(error) = result {
        debug_log!("peer {:?}: {error:#}", addr);
    }
    let useful =
        link.downloaded.load(Ordering::Relaxed) > 0 || link.uploaded.load(Ordering::Relaxed) > 0;
    drop(reader);
    useful
}

async fn read_loop(read_half: &mut ReadHalf<BoxedStream>, reader: &mut Reader) -> Result<()> {
    let link = reader.link.clone();
    let mut buffer = BytesMut::with_capacity(READ_BUFFER);
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        while let Some(len) = frame_len(&buffer, reader.max_frame)? {
            let frame = buffer.split_to(len).freeze();
            reader.handle_frame(frame).await?;
        }
        if !reader.after_messages() {
            return Ok(());
        }
        buffer.reserve(READ_BUFFER / 2);
        tokio::select! {
            read = read_half.read_buf(&mut buffer) => {
                if read? == 0 {
                    bail!("peer closed the connection");
                }
                reader.last_message = Instant::now();
            }
            _ = link.reader_wake.notified() => {}
            _ = tick.tick() => reader.on_tick()?,
        }
    }
}

async fn write_loop(
    write_half: &mut WriteHalf<BoxedStream>,
    live: &Arc<Live>,
    link: &Arc<PeerLink>,
) -> Result<()> {
    let mut control = vec![0u8; MAX_MSG_LEN];
    let mut block = Vec::new();
    loop {
        let used = serialize_control(live, link, &mut control)?;
        if used > 0 {
            timeout(WRITE_TIMEOUT, write_half.write_all(&control[..used]))
                .await
                .context("timed out sending to a peer")??;
            continue;
        }
        let upload = link.outbox.lock().uploads.pop_front();
        let Some(request) = upload else {
            if timeout(KEEP_ALIVE, link.writer_wake.notified())
                .await
                .is_err()
            {
                link.push(Out::KeepAlive);
            }
            continue;
        };
        // Waiting for upload allowance can take seconds, and our requests to this same peer
        // must not wait with it, or a low upload limit would slow the download too.
        let allowance = live.core.context.up.take(request.length);
        tokio::pin!(allowance);
        loop {
            tokio::select! {
                _ = &mut allowance => break,
                _ = link.writer_wake.notified() => {
                    let used = serialize_control(live, link, &mut control)?;
                    timeout(WRITE_TIMEOUT, write_half.write_all(&control[..used]))
                        .await
                        .context("timed out sending to a peer")??;
                }
            }
        }
        block = read_block(live, request, block).await?;
        timeout(WRITE_TIMEOUT, write_half.write_all(&block))
            .await
            .context("timed out sending to a peer")??;
        live.core.count_uploaded(u64::from(request.length));
        link.uploaded
            .fetch_add(u64::from(request.length), Ordering::Relaxed);
    }
}

/// Reads a block from disk into a whole piece message. Hands the buffer back to be reused.
async fn read_block(live: &Arc<Live>, request: Request, mut block: Vec<u8>) -> Result<Vec<u8>> {
    let core = live.core.clone();
    let lengths = &core.meta.lengths;
    let piece = lengths
        .validate_piece_index(request.index)
        .context("bad piece number in a queued request")?;
    let offset = lengths.piece_offset(piece) + u64::from(request.begin);
    let total = PIECE_HEADER + request.length as usize;
    block.resize(total, 0);
    block[0..4].copy_from_slice(&(total as u32 - 4).to_be_bytes());
    block[4] = 7;
    block[5..9].copy_from_slice(&request.index.to_be_bytes());
    block[9..13].copy_from_slice(&request.begin.to_be_bytes());
    let storage = core.storage.clone();
    let (block, result) = tokio::task::spawn_blocking(move || {
        let result = storage.read(offset, &mut block[PIECE_HEADER..]);
        (block, result)
    })
    .await?;
    result?;
    Ok(block)
}

/// Puts waiting control messages into `buffer`, and returns how many bytes that made.
fn serialize_control(live: &Live, link: &PeerLink, buffer: &mut Vec<u8>) -> Result<usize> {
    let ids = *link.extended_ids.lock();
    let mut used = 0;
    while used < WRITE_BATCH {
        let out = link.outbox.lock().control.pop_front();
        let Some(out) = out else {
            break;
        };
        let needed = match &out {
            Out::Bitfield(bytes) => bytes.len() + 5,
            _ => MAX_MSG_LEN,
        };
        if buffer.len() < used + needed {
            buffer.resize(used + needed, 0);
        }
        used += serialize_one(live, link, &out, ids, &mut buffer[used..])?;
    }
    Ok(used)
}

fn serialize_one(
    live: &Live,
    link: &PeerLink,
    out: &Out,
    ids: PeerExtendedMessageIds,
    buffer: &mut [u8],
) -> Result<usize> {
    let info = &live.core.meta.info_bytes;
    let message = match out {
        Out::KeepAlive => Message::KeepAlive,
        Out::Choke => Message::Choke,
        Out::Unchoke => Message::Unchoke,
        Out::Interested => Message::Interested,
        Out::NotInterested => Message::NotInterested,
        Out::Have(piece) => Message::Have(*piece),
        Out::Bitfield(bytes) => Message::Bitfield(ByteBuf(bytes)),
        Out::Request(request) => Message::Request(*request),
        Out::Cancel(request) => Message::Cancel(*request),
        Out::ExtendedHandshake => {
            let mut handshake = ExtendedHandshake::new();
            if live.core.meta.private {
                handshake.m.ut_pex = None;
            }
            handshake.v = Some(ByteBuf(CLIENT_NAME.as_bytes()));
            handshake.reqq = Some(MAX_UPLOAD_QUEUE as u32);
            handshake.metadata_size = Some(info.len() as u32);
            handshake.p = live.core.options.listen_port.map(u32::from);
            Message::Extended(ExtendedMessage::Handshake(handshake))
        }
        Out::MetadataPiece(piece) => {
            link.outbox.lock().pending_metadata -= 1;
            let start = *piece as usize * CHUNK_SIZE as usize;
            let end = (start + CHUNK_SIZE as usize).min(info.len());
            Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Data(
                UtMetadataData::from_bytes(*piece, info.len() as u32, ByteBuf(&info[start..end])),
            )))
        }
        Out::MetadataReject(piece) => {
            link.outbox.lock().pending_metadata -= 1;
            Message::Extended(ExtendedMessage::UtMetadata(UtMetadata::Reject(*piece)))
        }
    };
    message
        .serialize(buffer, &|| ids)
        .map_err(|error| anyhow!("{error}"))
}

/// What came out of reading a frame, once the message borrowing it is gone.
enum Action {
    Nothing,
    Block { index: u32, begin: u32, data: Bytes },
}

struct Reader {
    pending_write: Option<tokio::task::JoinHandle<(io::Result<()>, crate::torrent::WriteLease)>>,
    live: Arc<Live>,
    link: Arc<PeerLink>,
    pieces: u32,
    max_frame: usize,
    peer_has: Bitfield,
    peer_choking: bool,
    am_interested: bool,
    in_flight: HashMap<Chunk, Instant>,
    /// Chunks given up on. If they arrive after all, they are still welcome.
    released: HashSet<Chunk>,
    peer_queue_limit: usize,
    queue_target: usize,
    unsolicited: u32,
    timeouts: u32,
    recent_bytes: u64,
    last_message: Instant,
}

impl Reader {
    fn new(live: Arc<Live>, link: Arc<PeerLink>) -> Self {
        let pieces = live.core.meta.lengths.total_pieces();
        let bitfield_message = pieces.div_ceil(8) as usize + 5;
        Self {
            pending_write: None,
            live,
            link,
            pieces,
            max_frame: bitfield_message.max(MAX_MSG_LEN),
            peer_has: Bitfield::new(pieces),
            peer_choking: true,
            am_interested: false,
            in_flight: HashMap::new(),
            released: HashSet::new(),
            peer_queue_limit: MAX_QUEUE,
            queue_target: MIN_QUEUE,
            unsolicited: 0,
            timeouts: 0,
            recent_bytes: 0,
            last_message: Instant::now(),
        }
    }

    async fn handle_frame(&mut self, frame: Bytes) -> Result<()> {
        let action = self.parse_frame(&frame)?;
        if let Action::Block { index, begin, data } = action {
            self.on_block(index, begin, data).await?;
        }
        Ok(())
    }

    fn parse_frame(&mut self, frame: &Bytes) -> Result<Action> {
        let message = match Message::deserialize(frame, &[]) {
            Ok((message, _)) => message,
            // Messages for extensions we didn't ask for can simply be skipped.
            Err(MessageDeserializeError::UnsupportedMessageId(_)) => return Ok(Action::Nothing),
            Err(error) => bail!("bad message: {error}"),
        };
        match message {
            Message::KeepAlive => {}
            Message::Choke => {
                self.peer_choking = true;
                self.link.peer_choking.store(true, Ordering::Relaxed);
                self.release_all();
            }
            Message::Unchoke => {
                self.peer_choking = false;
                self.link.peer_choking.store(false, Ordering::Relaxed);
            }
            Message::Interested => {
                self.link.interested.store(true, Ordering::Release);
                self.live.try_unchoke(&self.link);
            }
            Message::NotInterested => {
                self.link.interested.store(false, Ordering::Release);
            }
            Message::Have(piece) => self.on_have(piece)?,
            Message::Bitfield(bytes) => self.on_bitfield(bytes.0)?,
            Message::Request(request) => self.on_request(request)?,
            Message::Cancel(request) => self.link.cancel_upload(request),
            Message::Piece(piece) => {
                let (first, second) = piece.data();
                ensure!(second.is_empty(), "a block arrived in two parts");
                return Ok(Action::Block {
                    index: piece.index,
                    begin: piece.begin,
                    data: frame.slice_ref(first),
                });
            }
            Message::Extended(extended) => self.on_extended(extended)?,
        }
        Ok(Action::Nothing)
    }

    fn on_have(&mut self, piece: u32) -> Result<()> {
        ensure!(
            piece < self.pieces,
            "peer claims a piece that doesn't exist"
        );
        if self.peer_has.set(piece) {
            self.live.core.picker.lock().peer_has_piece(piece);
            self.link.has_pieces.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn on_bitfield(&mut self, bytes: &[u8]) -> Result<()> {
        let field = Bitfield::from_wire(bytes, self.pieces)
            .context("peer sent a bitfield of the wrong size")?;
        let mut picker = self.live.core.picker.lock();
        picker.peer_removed(&self.peer_has);
        picker.peer_added(&field);
        drop(picker);
        self.link.has_pieces.store(field.count(), Ordering::Relaxed);
        self.peer_has = field;
        Ok(())
    }

    fn on_request(&mut self, request: Request) -> Result<()> {
        let core = &self.live.core;
        let lengths = &core.meta.lengths;
        let piece = lengths
            .validate_piece_index(request.index)
            .context("peer asked for a piece that doesn't exist")?;
        let end = request
            .begin
            .checked_add(request.length)
            .context("peer asked for a block past the end of a piece")?;
        ensure!(
            request.length > 0
                && request.length <= CHUNK_SIZE
                && end <= lengths.piece_length(piece),
            "peer asked for a block that doesn't fit its piece"
        );
        if !self.link.is_choking() && core.picker.lock().have().get(request.index) {
            self.link.queue_upload(request);
        }
        Ok(())
    }

    fn on_extended(&mut self, message: ExtendedMessage<ByteBuf<'_>>) -> Result<()> {
        match message {
            ExtendedMessage::Handshake(handshake) => {
                if let Some(client) = &handshake.v {
                    let client = String::from_utf8_lossy(client.as_ref());
                    debug_log!("peer {:?} runs {}", self.link.addr, client.escape_debug());
                    *self.link.client.lock() = clean_text(&client, MAX_CLIENT_CHARS);
                }
                *self.link.extended_ids.lock() = handshake.peer_extended_messages();
                if let Some(limit) = handshake.reqq {
                    self.peer_queue_limit = (limit as usize).clamp(1, MAX_QUEUE);
                    self.queue_target = self.queue_target.min(self.peer_queue_limit);
                }
            }
            ExtendedMessage::UtMetadata(UtMetadata::Request(piece)) => {
                self.on_metadata_request(piece)
            }
            ExtendedMessage::UtPex(pex) if !self.live.core.meta.private => {
                let found = pex.added_peers().take(MAX_PEX_PEERS).map(|peer| peer.addr);
                self.live.discovered(found);
            }
            _ => {}
        }
        Ok(())
    }

    fn on_metadata_request(&self, piece: u32) {
        if self.link.extended_ids.lock().ut_metadata.is_none() {
            return;
        }
        let size = self.live.core.meta.info_bytes.len() as u32;
        if piece < size.div_ceil(CHUNK_SIZE) && !self.live.core.uploads_off() {
            self.link.queue_metadata(Out::MetadataPiece(piece));
        } else {
            self.link.queue_metadata(Out::MetadataReject(piece));
        }
    }

    async fn on_block(&mut self, index: u32, begin: u32, data: Bytes) -> Result<()> {
        let core = self.live.core.clone();
        let lengths = &core.meta.lengths;
        let piece = lengths
            .validate_piece_index(index)
            .context("peer sent a block for a piece that doesn't exist")?;
        let info = lengths
            .chunk_info_from_received_data(piece, begin, data.len() as u32)
            .context("peer sent a block that doesn't match any request")?;
        let chunk = Chunk {
            piece: index,
            index: info.chunk_index,
        };
        let expected = self.in_flight.remove(&chunk).is_some() | self.released.remove(&chunk);
        if !expected {
            self.unsolicited += 1;
            ensure!(
                self.unsolicited <= MAX_UNSOLICITED,
                "peer keeps sending blocks nobody asked for"
            );
            return Ok(());
        }
        let size = data.len() as u64;
        self.recent_bytes += size;
        self.timeouts = 0;
        if !core.picker.lock().wants_chunk(chunk) {
            core.count_downloaded(size);
            core.wasted.fetch_add(size, Ordering::Relaxed);
            return Ok(());
        }
        core.context.down.take(size as u32).await;
        core.options.allowance.take(size as u32).await;

        let Some(write_lease) = core.lease_write(chunk) else {
            core.count_downloaded(size);
            core.wasted.fetch_add(size, Ordering::Relaxed);
            return Ok(());
        };
        let offset = lengths.chunk_absolute_offset(&info);
        let storage = core.storage.clone();
        self.pending_write = Some(tokio::task::spawn_blocking(move || {
            (storage.write(offset, &data), write_lease)
        }));
        let written = self.pending_write.as_mut().unwrap().await;
        self.pending_write.take();
        let (written, _write_lease) = written?;
        if let Err(error) = written {
            let message = format!("Could not write to the download folder: {error}");
            let _ = self.live.events.send(Event::Fatal(message));
            bail!("write failed: {error}");
        }

        core.count_downloaded(size);
        self.link.downloaded.fetch_add(size, Ordering::Relaxed);
        let (outcome, copies) = {
            let mut picker = core.picker.lock();
            let copies = picker.copies_out(chunk);
            (picker.chunk_written(chunk, self.link.key), copies)
        };
        match outcome {
            Written::Duplicate => {
                core.wasted.fetch_add(size, Ordering::Relaxed);
            }
            Written::Stored => {}
            Written::PieceComplete => {
                let _ = self.live.events.send(Event::PieceComplete(index));
            }
        }
        if copies > 1 {
            self.live.chunk_finished(chunk, self.link.key);
        }
        Ok(())
    }

    /// Runs after each batch of messages, and whenever something else changes. Returns
    /// false when this connection has no more use.
    fn after_messages(&mut self) -> bool {
        self.drop_finished_chunks();
        self.fill_requests();
        let picker = self.live.core.picker.lock();
        // A seed needs nothing from us, and once we're done we need nothing from it.
        !(picker.is_finished() && self.peer_has.is_full())
    }

    fn drop_finished_chunks(&mut self) {
        let finished = std::mem::take(&mut *self.link.finished_chunks.lock());
        for chunk in finished {
            if self.in_flight.remove(&chunk).is_some()
                && let Some(request) = self.live.core.request_for(chunk)
            {
                self.link.push(Out::Cancel(request));
                // The block may already be on its way, which is no fault of the peer's.
                self.remember_released(chunk);
            }
        }
    }

    fn remember_released(&mut self, chunk: Chunk) {
        if self.released.len() >= MAX_RELEASED {
            self.released.clear();
        }
        self.released.insert(chunk);
    }

    fn fill_requests(&mut self) {
        let core = &self.live.core;
        let mut picker = core.picker.lock();
        let interesting = !core.options.seed_only && picker.is_interesting(&self.peer_has);
        if interesting != self.am_interested {
            self.am_interested = interesting;
            self.link
                .am_interested
                .store(interesting, Ordering::Relaxed);
            self.link.push(if interesting {
                Out::Interested
            } else {
                Out::NotInterested
            });
        }
        if !interesting || self.peer_choking {
            return;
        }
        let room = self.queue_target.saturating_sub(self.in_flight.len());
        // Waiting for a little room avoids taking the lock for each block.
        if room == 0 || (room < self.queue_target / 4 && !self.in_flight.is_empty()) {
            return;
        }
        let in_flight = &self.in_flight;
        let requests = picker.next_requests(
            &self.peer_has,
            &|chunk| in_flight.contains_key(&chunk),
            room,
            std::time::Instant::now(),
        );
        drop(picker);
        let now = Instant::now();
        for request in requests {
            self.in_flight.insert(request.chunk, now);
            self.link.push(Out::Request(Request::new(
                request.chunk.piece,
                request.begin,
                request.length,
            )));
        }
    }

    /// Gives every open request back, so that other peers can be asked.
    fn release_all(&mut self) {
        if self.in_flight.is_empty() {
            return;
        }
        let given_up: Vec<Chunk> = self.in_flight.drain().map(|(chunk, _)| chunk).collect();
        let mut picker = self.live.core.picker.lock();
        for chunk in &given_up {
            picker.release(*chunk);
        }
        drop(picker);
        for chunk in given_up {
            self.remember_released(chunk);
        }
        self.live.nudge_all();
    }

    fn on_tick(&mut self) -> Result<()> {
        let now = Instant::now();
        ensure!(
            now.duration_since(self.last_message) < IDLE_TIMEOUT,
            "peer went quiet"
        );
        let oldest = self.in_flight.values().min().copied();
        if let Some(oldest) = oldest
            && now.duration_since(oldest) > REQUEST_TIMEOUT
        {
            self.timeouts += 1;
            ensure!(self.timeouts < MAX_TIMEOUTS, "peer stopped answering");
            self.release_all();
        }
        // Keep about two seconds of this peer's speed in requests.
        let per_second = self.recent_bytes / TICK.as_secs();
        self.recent_bytes = 0;
        self.queue_target = ((per_second * 2 / u64::from(CHUNK_SIZE)) as usize)
            .clamp(MIN_QUEUE, MAX_QUEUE)
            .min(self.peer_queue_limit);
        Ok(())
    }
}

impl Drop for Reader {
    /// However a connection ends, what it held goes back, or those chunks would stay
    /// "requested" forever and the download could never finish.
    fn drop(&mut self) {
        {
            let mut picker = self.live.core.picker.lock();
            picker.peer_removed(&self.peer_has);
            for chunk in self.in_flight.keys() {
                picker.release(*chunk);
            }
        }
        self.live.forget(self.link.key);
        self.live.nudge_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_frame_needs_its_whole_length() {
        assert_eq!(frame_len(&[0, 0], 100).unwrap(), None);
        assert_eq!(frame_len(&[0, 0, 0, 2, 1], 100).unwrap(), None);
        assert_eq!(frame_len(&[0, 0, 0, 2, 1, 2], 100).unwrap(), Some(6));
        assert_eq!(frame_len(&[0, 0, 0, 0], 100).unwrap(), Some(4));
    }

    #[test]
    fn test_a_huge_announced_length_is_refused_at_once() {
        assert!(frame_len(&[0xff, 0xff, 0xff, 0xff], MAX_MSG_LEN).is_err());
        assert!(frame_len(&[0, 0, 1, 0], 100).is_err());
    }

    #[tokio::test]
    async fn test_handshakes_round_trip() {
        let (mut left, mut right) = tokio::io::duplex(256);
        let hash = Id20::new([7; 20]);
        let id = Id20::new([9; 20]);
        send_handshake(&mut left, hash, id).await.unwrap();
        let (seen_hash, hello) = read_handshake(&mut right).await.unwrap();
        assert_eq!(seen_hash, hash);
        assert_eq!(hello.peer_id, id);
        assert!(hello.supports_extended);
    }

    #[tokio::test]
    async fn test_a_bad_handshake_is_refused() {
        let (mut left, mut right) = tokio::io::duplex(256);
        left.write_all(&[0u8; HANDSHAKE_LEN]).await.unwrap();
        assert!(read_handshake(&mut right).await.is_err());
    }
}
