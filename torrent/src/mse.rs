//! BitTorrent Message Stream Encryption (MSE/PE), a legacy peer-wire obfuscation layer.
//! It is NOT authenticated encryption, anonymity, or a replacement for TLS/a VPN.
//! Wire format: https://tixati.com/specs/bittorrent/peer_connections/crypto
//! The RC4 discard is 1024 BYTES (as in libtorrent), not the typo in that description.

use crate::peer::BoxedStream;
use anyhow::{Context as _, Result, ensure};
use bytes::{Buf, Bytes};
use librqbit_core::hash_id::Id20;
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};
use sha1w::{ISha1, Sha1};
use std::{
    io,
    pin::Pin,
    sync::{Arc, OnceLock},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};
use zeroize::{Zeroize, Zeroizing};

const PRIME: &[u8] = b"FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A63A36210000000000090563";
const PREFIX: &[u8; 20] = b"\x13BitTorrent protocol";
const MAX_PAD: usize = 512;
const DEADLINE: Duration = Duration::from_secs(15);
static CRYPTO_WORK: OnceLock<Arc<Semaphore>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encryption {
    #[default]
    Disabled,
    Prefer,
    Required,
}
impl Encryption {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Prefer => "prefer",
            Self::Required => "required",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "prefer" => Ok(Self::Prefer),
            "required" => Ok(Self::Required),
            _ => anyhow::bail!("Unknown peer encryption mode"),
        }
    }
}

fn digest(parts: &[&[u8]]) -> [u8; 20] {
    let mut hash = Sha1::new();
    for part in parts {
        hash.update(part);
    }
    hash.finish()
}
fn prime() -> &'static BigUint {
    static VALUE: OnceLock<BigUint> = OnceLock::new();
    VALUE.get_or_init(|| BigUint::parse_bytes(PRIME, 16).expect("constant MSE prime"))
}
fn fixed(value: BigUint) -> [u8; 96] {
    let bytes = value.to_bytes_be();
    let mut result = [0; 96];
    result[96 - bytes.len()..].copy_from_slice(&bytes);
    result
}
struct Dh {
    secret: Zeroizing<[u8; 20]>,
}
impl Dh {
    fn new() -> Result<(Self, [u8; 96])> {
        let mut secret = Zeroizing::new([0; 20]);
        getrandom::fill(secret.as_mut())
            .map_err(|_| anyhow::anyhow!("Could not generate a peer handshake key"))?;
        // Never accept an all-zero exponent, even if the entropy source malfunctions.
        ensure!(
            secret.iter().any(|b| *b != 0),
            "Invalid peer handshake randomness"
        );
        let public =
            fixed(BigUint::from(2u8).modpow(&BigUint::from_bytes_be(secret.as_ref()), prime()));
        Ok((Self { secret }, public))
    }
    fn shared(self, peer: [u8; 96]) -> Result<Zeroizing<[u8; 96]>> {
        let peer = BigUint::from_bytes_be(&peer);
        ensure!(
            peer >= BigUint::from(2u8) && peer < prime() - 1u8,
            "Invalid peer public key"
        );
        let shared = peer.modpow(&BigUint::from_bytes_be(self.secret.as_ref()), prime());
        ensure!(shared > BigUint::from(1u8), "Invalid shared peer key");
        Ok(Zeroizing::new(fixed(shared)))
    }
}

// The permit moves into blocking work, so cancellation cannot leave uncounted CPU jobs.
async fn new_dh() -> Result<(Dh, [u8; 96], OwnedSemaphorePermit)> {
    let permit = CRYPTO_WORK
        .get_or_init(|| Arc::new(Semaphore::new(4)))
        .clone()
        .try_acquire_owned()
        .context("Peer encryption handshake capacity is full")?;
    tokio::task::spawn_blocking(move || Dh::new().map(|(dh, public)| (dh, public, permit))).await?
}
async fn shared(
    dh: Dh,
    peer: [u8; 96],
    permit: OwnedSemaphorePermit,
) -> Result<Zeroizing<[u8; 96]>> {
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        dh.shared(peer)
    })
    .await?
}

#[derive(Clone)]
struct Rc4 {
    state: [u8; 256],
    i: u8,
    j: u8,
}
impl Drop for Rc4 {
    fn drop(&mut self) {
        self.state.zeroize();
        self.i.zeroize();
        self.j.zeroize();
    }
}
impl Rc4 {
    fn new(key: &[u8]) -> Self {
        assert!(!key.is_empty());
        let mut state = [0; 256];
        for (i, value) in state.iter_mut().enumerate() {
            *value = i as u8;
        }
        let mut j = 0u8;
        for i in 0..256 {
            j = j.wrapping_add(state[i]).wrapping_add(key[i % key.len()]);
            state.swap(i, j as usize);
        }
        Self { state, i: 0, j: 0 }
    }
    fn apply(&mut self, bytes: &mut [u8]) {
        for byte in bytes {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.state[self.i as usize]);
            self.state.swap(self.i as usize, self.j as usize);
            *byte ^= self.state
                [self.state[self.i as usize].wrapping_add(self.state[self.j as usize]) as usize];
        }
    }
    fn discard(&mut self, mut count: usize) {
        let mut scratch = [0; 256];
        while count > 0 {
            let n = count.min(scratch.len());
            self.apply(&mut scratch[..n]);
            count -= n;
        }
        scratch.zeroize();
    }
    fn peer_key(label: &[u8], shared: &[u8], hash: Id20) -> Self {
        let key = Zeroizing::new(digest(&[label, shared, &hash.0]));
        let mut cipher = Self::new(key.as_ref());
        cipher.discard(1024);
        cipher
    }
}

struct Encrypted {
    inner: BoxedStream,
    rx: Rc4,
    tx: Rc4,
    prefix: Bytes,
}
impl AsyncRead for Encrypted {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.prefix.is_empty() {
            let n = out.remaining().min(this.prefix.len());
            out.put_slice(&this.prefix[..n]);
            this.prefix.advance(n);
            return Poll::Ready(Ok(()));
        }
        let before = out.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, out);
        this.rx.apply(&mut out.filled_mut()[before..]);
        result
    }
}
impl AsyncWrite for Encrypted {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        input: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let n = input.len().min(16 * 1024);
        let mut scratch = [0; 16 * 1024];
        scratch[..n].copy_from_slice(&input[..n]);
        let mut candidate = this.tx.clone();
        candidate.apply(&mut scratch[..n]);
        let result = Pin::new(&mut this.inner).poll_write(cx, &scratch[..n]);
        // Pending/cancelled writes must not consume keystream. Partial writes consume only
        // the accepted bytes; no hidden buffered uploads survive Pause/Stop.
        if let Poll::Ready(Ok(count)) = result {
            this.tx.discard(count);
        }
        result
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
struct Prefixed {
    prefix: Bytes,
    inner: BoxedStream,
}
impl AsyncRead for Prefixed {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.prefix.is_empty() {
            let n = out.remaining().min(this.prefix.len());
            out.put_slice(&this.prefix[..n]);
            this.prefix.advance(n);
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut this.inner).poll_read(cx, out)
        }
    }
}
impl AsyncWrite for Prefixed {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        input: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, input)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
async fn send_public(stream: &mut BoxedStream, public: &[u8; 96]) -> Result<()> {
    let mut random = [0; MAX_PAD + 2];
    getrandom::fill(&mut random)
        .map_err(|_| anyhow::anyhow!("Could not generate handshake padding"))?;
    let count = (u16::from_be_bytes([random[0], random[1]]) as usize) % (MAX_PAD + 1);
    stream.write_all(public).await?;
    stream.write_all(&random[2..2 + count]).await?;
    Ok(())
}
async fn synchronize(stream: &mut BoxedStream, marker: &[u8]) -> Result<()> {
    let mut window = vec![0; marker.len()];
    stream.read_exact(&mut window).await?;
    for skipped in 0..=MAX_PAD {
        if window == marker {
            return Ok(());
        }
        ensure!(
            skipped < MAX_PAD,
            "Peer encryption padding exceeds the limit"
        );
        window.rotate_left(1);
        *window.last_mut().unwrap() = stream.read_u8().await?;
    }
    unreachable!()
}
async fn decrypted<const N: usize>(stream: &mut BoxedStream, cipher: &mut Rc4) -> Result<[u8; N]> {
    let mut out = [0; N];
    stream.read_exact(&mut out).await?;
    cipher.apply(&mut out);
    Ok(out)
}
async fn discard_pad(stream: &mut BoxedStream, cipher: &mut Rc4, size: u16) -> Result<()> {
    ensure!(
        size as usize <= MAX_PAD,
        "Peer encryption padding exceeds the limit"
    );
    let mut pad = [0; MAX_PAD];
    stream.read_exact(&mut pad[..size as usize]).await?;
    cipher.apply(&mut pad[..size as usize]);
    Ok(())
}

pub(crate) async fn outgoing(mut stream: BoxedStream, hash: Id20) -> Result<BoxedStream> {
    timeout(DEADLINE, async move {
        let (dh, public, permit) = new_dh().await?;
        send_public(&mut stream, &public).await?;
        let mut peer = [0; 96];
        stream.read_exact(&mut peer).await?;
        let shared = shared(dh, peer, permit).await?;
        let mut tx = Rc4::peer_key(b"keyA", shared.as_ref(), hash);
        let mut rx = Rc4::peer_key(b"keyB", shared.as_ref(), hash);
        stream
            .write_all(&digest(&[b"req1", shared.as_ref()]))
            .await?;
        let mut masked = digest(&[b"req2", &hash.0]);
        for (a, b) in masked.iter_mut().zip(digest(&[b"req3", shared.as_ref()])) {
            *a ^= b;
        }
        stream.write_all(&masked).await?;
        let mut offer = [0; 16];
        offer[11] = 2; // RC4 only, no PadC or initial payload.
        tx.apply(&mut offer);
        stream.write_all(&offer).await?;
        let mut marker = [0; 8];
        rx.apply(&mut marker);
        synchronize(&mut stream, &marker).await?;
        let select = u32::from_be_bytes(decrypted(&mut stream, &mut rx).await?);
        ensure!(select == 2, "Peer did not select encrypted transfer");
        let padding = u16::from_be_bytes(decrypted(&mut stream, &mut rx).await?);
        discard_pad(&mut stream, &mut rx, padding).await?;
        Ok(Box::new(Encrypted {
            inner: stream,
            rx,
            tx,
            prefix: Bytes::new(),
        }) as BoxedStream)
    })
    .await
    .context("Peer encryption handshake timed out")?
}

/// Returns the MSE-selected hash, if encrypted. The caller must compare it with the
/// BitTorrent handshake's hash before offering the stream to that torrent.
pub(crate) async fn incoming(
    mut stream: BoxedStream,
    mode: Encryption,
    hashes: Vec<Id20>,
) -> Result<(BoxedStream, Option<Id20>)> {
    timeout(DEADLINE, async move {
        let mut first = [0; 20];
        stream.read_exact(&mut first).await?;
        if &first == PREFIX {
            ensure!(
                mode != Encryption::Required,
                "Unencrypted peer refused by encryption policy"
            );
            return Ok((
                Box::new(Prefixed {
                    inner: stream,
                    prefix: Bytes::copy_from_slice(&first),
                }) as BoxedStream,
                None,
            ));
        }
        ensure!(mode != Encryption::Disabled, "Encrypted peers are disabled");
        ensure!(
            hashes.len() <= 4096,
            "Too many torrents for a peer encryption handshake"
        );
        let mut peer = [0; 96];
        peer[..20].copy_from_slice(&first);
        stream.read_exact(&mut peer[20..]).await?;
        let (dh, public, permit) = new_dh().await?;
        let shared = shared(dh, peer, permit).await?;
        send_public(&mut stream, &public).await?;
        synchronize(&mut stream, &digest(&[b"req1", shared.as_ref()])).await?;
        let mut wanted = [0; 20];
        stream.read_exact(&mut wanted).await?;
        for (a, b) in wanted.iter_mut().zip(digest(&[b"req3", shared.as_ref()])) {
            *a ^= b;
        }
        let hash = hashes
            .into_iter()
            .find(|hash| digest(&[b"req2", &hash.0]) == wanted)
            .context("Peer asked for an unknown torrent")?;
        let mut rx = Rc4::peer_key(b"keyA", shared.as_ref(), hash);
        let mut tx = Rc4::peer_key(b"keyB", shared.as_ref(), hash);
        let vc: [u8; 8] = decrypted(&mut stream, &mut rx).await?;
        ensure!(vc == [0; 8], "Invalid peer encryption verification");
        let offer = u32::from_be_bytes(decrypted(&mut stream, &mut rx).await?);
        ensure!(offer & 2 != 0, "Peer did not offer encrypted transfer");
        let padding = u16::from_be_bytes(decrypted(&mut stream, &mut rx).await?);
        discard_pad(&mut stream, &mut rx, padding).await?;
        let initial_len = u16::from_be_bytes(decrypted(&mut stream, &mut rx).await?) as usize;
        // A u16 limits initial payload to 64 KiB, under the handshake admission budget.
        let mut initial = vec![0; initial_len];
        stream.read_exact(&mut initial).await?;
        rx.apply(&mut initial);
        let mut selection = [0; 14];
        selection[11] = 2;
        tx.apply(&mut selection);
        stream.write_all(&selection).await?;
        Ok((
            Box::new(Encrypted {
                inner: stream,
                rx,
                tx,
                prefix: Bytes::from(initial),
            }) as BoxedStream,
            Some(hash),
        ))
    })
    .await
    .context("Peer encryption handshake timed out")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::sample_bytes;

    #[test]
    fn rc4_matches_rfc6229_and_split_writes() {
        let mut cipher = Rc4::new(&[1, 2, 3, 4, 5]);
        let mut output = [0; 16];
        cipher.apply(&mut output);
        assert_eq!(
            output,
            [
                0xb2, 0x39, 0x63, 0x05, 0xf0, 0x3d, 0xc0, 0x27, 0xcc, 0xc3, 0x52, 0x4a, 0x0a, 0x11,
                0x18, 0xa8
            ]
        );
        let mut whole = sample_bytes(4000, 12);
        let mut split = whole.clone();
        Rc4::new(b"test").apply(&mut whole);
        let mut cipher = Rc4::new(b"test");
        for part in split.chunks_mut(7) {
            cipher.apply(part);
        }
        assert_eq!(whole, split);
    }

    #[test]
    fn degenerate_public_keys_are_refused() {
        for key in [
            BigUint::from(0u8),
            BigUint::from(1u8),
            prime() - 1u8,
            prime().clone(),
        ] {
            let (dh, _) = Dh::new().unwrap();
            assert!(dh.shared(fixed(key)).is_err());
        }
    }

    #[tokio::test]
    async fn encrypted_handshake_and_bidirectional_large_payloads() {
        let hash = Id20::new([9; 20]);
        let (a, b) = tokio::io::duplex(1024);
        let (a, b) = tokio::join!(
            outgoing(Box::new(a), hash),
            incoming(Box::new(b), Encryption::Required, vec![hash])
        );
        let mut a = a.unwrap();
        let (mut b, selected) = b.unwrap();
        assert_eq!(selected, Some(hash));
        let data = sample_bytes(200_003, 66);
        let (sent, received) = tokio::join!(
            async {
                a.write_all(&data).await.unwrap();
                a.flush().await.unwrap();
            },
            async {
                let mut got = vec![0; data.len()];
                b.read_exact(&mut got).await.unwrap();
                got
            }
        );
        let _ = sent;
        assert_eq!(received, data);
        let ((), received) = tokio::join!(
            async {
                b.write_all(&data).await.unwrap();
            },
            async {
                let mut got = vec![0; data.len()];
                a.read_exact(&mut got).await.unwrap();
                got
            }
        );
        assert_eq!(received, data);
    }

    #[tokio::test]
    async fn partial_pending_and_cancelled_writes_do_not_reuse_or_skip_keystream() {
        let (a, mut b) = tokio::io::duplex(7);
        let mut stream = Encrypted {
            inner: Box::new(a),
            tx: Rc4::new(b"test"),
            rx: Rc4::new(b"unused"),
            prefix: Bytes::new(),
        };
        stream.write_all(b"1234567").await.unwrap();
        assert!(
            timeout(Duration::from_millis(10), stream.write_all(b"cancelled"))
                .await
                .is_err()
        );
        let mut got = [0; 14];
        b.read_exact(&mut got[..7]).await.unwrap();
        stream.write_all(b"abcdefg").await.unwrap();
        b.read_exact(&mut got[7..]).await.unwrap();
        Rc4::new(b"test").apply(&mut got);
        assert_eq!(&got, b"1234567abcdefg");
    }

    #[tokio::test]
    async fn plaintext_required_is_refused_and_prefer_preserves_prefix() {
        for mode in [
            Encryption::Required,
            Encryption::Prefer,
            Encryption::Disabled,
        ] {
            let (mut a, b) = tokio::io::duplex(128);
            a.write_all(PREFIX).await.unwrap();
            let result = incoming(Box::new(b), mode, vec![]).await;
            if mode == Encryption::Required {
                assert!(result.is_err());
            } else {
                let (mut b, hash) = result.unwrap();
                let mut first = [0; 20];
                b.read_exact(&mut first).await.unwrap();
                assert_eq!(&first, PREFIX);
                assert!(hash.is_none());
            }
        }
    }

    #[tokio::test]
    async fn unknown_torrent_and_excessive_padding_are_refused() {
        let (a, b) = tokio::io::duplex(1024);
        let (a, b) = tokio::join!(
            outgoing(Box::new(a), Id20::new([1; 20])),
            incoming(Box::new(b), Encryption::Required, vec![Id20::new([2; 20])])
        );
        assert!(a.is_err());
        assert!(b.is_err());
        let (mut a, b) = tokio::io::duplex(1024);
        a.write_all(&[1; MAX_PAD + 21]).await.unwrap();
        let mut b: BoxedStream = Box::new(b);
        assert!(synchronize(&mut b, &[2; 20]).await.is_err());
        assert!(
            discard_pad(&mut b, &mut Rc4::new(b"test"), 513)
                .await
                .is_err()
        );
    }
}
