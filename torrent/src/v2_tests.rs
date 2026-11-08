use crate::{meta::TorrentMeta, storage::Storage, testkit::TempFolder, v2::*, v2_wire::Request};
use std::sync::Arc;

fn b(bytes: &[u8]) -> Vec<u8> {
    let mut v = format!("{}:", bytes.len()).into_bytes();
    v.extend(bytes);
    v
}
fn i(n: usize) -> Vec<u8> {
    format!("i{n}e").into_bytes()
}
fn d(mut entries: Vec<(&[u8], Vec<u8>)>) -> Vec<u8> {
    entries.sort_by_key(|e| e.0);
    let mut v = vec![b'd'];
    for (k, x) in entries {
        v.extend(b(k));
        v.extend(x);
    }
    v.push(b'e');
    v
}

fn sample(data: &[u8], piece: usize) -> Vec<u8> {
    let mut leaves: Vec<_> = data.chunks(BLOCK as usize).map(digest).collect();
    let file_root = root(leaves.clone(), [0; 32]);
    let layer = (piece / BLOCK as usize).ilog2();
    let count = data.len().div_ceil(piece);
    leaves.resize(
        data.len().div_ceil(piece) * (piece / BLOCK as usize),
        [0; 32],
    );
    let hashes: Vec<u8> = leaves
        .chunks(piece / BLOCK as usize)
        .flat_map(|l| root(l.to_vec(), [0; 32]))
        .collect();
    let info = d(vec![
        (
            b"file tree",
            d(vec![(
                b"sample.bin",
                d(vec![(
                    b"",
                    d(vec![
                        (b"length", i(data.len())),
                        (b"pieces root", b(&file_root)),
                    ]),
                )]),
            )]),
        ),
        (b"meta version", i(2)),
        (b"name", b(b"sample.bin")),
        (b"piece length", i(piece)),
    ]);
    assert_eq!(layer, (piece as u32 / BLOCK).ilog2());
    d(vec![
        (b"info", info),
        (
            b"piece layers",
            if count > 1 {
                d(vec![(&file_root, b(&hashes))])
            } else {
                d(vec![])
            },
        ),
    ])
}

#[test]
fn v2_payload_hashes_cover_short_blocks_and_merkle_padding() {
    for piece in [16384, 32768, 131072] {
        for len in [1, 16383, 16384, 16385, 65539, 200013] {
            let data: Vec<_> = (0..len).map(|n| (n % 251) as u8).collect();
            let meta = Arc::new(TorrentMeta::from_torrent_file(&sample(&data, piece)).unwrap());
            assert!(meta.hashes.v1.is_none());
            assert_eq!(meta.hashes.key().len(), 64);
            let folder = TempFolder::new();
            let storage = Storage::new(meta.clone(), folder.path().to_owned());
            storage.write(0, &data).unwrap();
            for index in 0..meta.lengths.total_pieces() {
                assert!(
                    storage.piece_matches(index).unwrap(),
                    "piece={piece} len={len} index={index}"
                );
            }
            storage.write(0, &[255]).unwrap();
            assert!(!storage.piece_matches(0).unwrap());
        }
    }
}

#[test]
fn v2_rejects_tampered_layers_and_metadata() {
    let data = vec![17; 100001];
    let torrent = sample(&data, 32768);
    let meta = TorrentMeta::from_torrent_file(&torrent).unwrap();
    let mut corrupt = torrent.clone();
    let last = corrupt.len() - 3;
    corrupt[last] ^= 1;
    assert!(TorrentMeta::from_torrent_file(&corrupt).is_err());
    let mut info = meta.info_bytes.to_vec();
    info[10] ^= 1;
    assert!(TorrentMeta::from_hashes(meta.hashes, info.into(), vec![]).is_err());
    assert!(TorrentMeta::from_info_bytes(meta.info_hash, meta.info_bytes.clone(), vec![]).is_err());
    let untrusted = TorrentMeta::from_hashes(meta.hashes, meta.info_bytes.clone(), vec![]).unwrap();
    assert!(untrusted.v2[0].hashes.is_empty());
    let folder = TempFolder::new();
    let store = Storage::new(Arc::new(untrusted), folder.path().to_owned());
    store.write(0, &data).unwrap();
    assert!(!store.piece_matches(0).unwrap());
    let untrusted =
        Arc::new(TorrentMeta::from_hashes(meta.hashes, meta.info_bytes.clone(), vec![]).unwrap());
    assert!(
        crate::torrent::Torrent::from_meta(
            untrusted,
            crate::testkit::options_for(folder.path()),
            crate::testkit::context(1)
        )
        .is_err()
    );
}

#[test]
fn v2_hash_proofs_authenticate_and_reject_unsolicited_responses() {
    let data = vec![21; 900123];
    let meta = Arc::new(TorrentMeta::from_torrent_file(&sample(&data, 32768)).unwrap());
    let file = &meta.v2[0];
    let folder = TempFolder::new();
    let storage = Storage::new(meta.clone(), folder.path().to_owned());
    storage.write(0, &data).unwrap();
    let have = meta.wanted_pieces(None).unwrap();
    let request = Request {
        root: file.root,
        base: file.layer,
        index: 0,
        length: 8,
        proof: file.height() - file.layer - 1,
    };
    let response = crate::v2_wire::answer(request, &storage, &meta, &have).unwrap();
    assert_eq!(request.verify(file, &response).unwrap(), file.hashes[..8]);
    let mut corrupt = response.to_vec();
    corrupt[54] ^= 1;
    assert!(request.verify(file, &corrupt).is_err());
    let other = Request {
        index: 8,
        ..request
    };
    assert!(other.verify(file, &response).is_err());
    assert!(
        request
            .verify(file, &response[..response.len() - 32])
            .is_err()
    );
    for bad in [
        Request {
            length: 1024,
            ..request
        },
        Request {
            index: 1,
            ..request
        },
        Request {
            proof: 1000,
            ..request
        },
        Request {
            base: 1000,
            ..request
        },
    ] {
        assert!(Request::parse(&bad.frame(21, &[])).is_err());
    }
    let leaf = Request {
        base: 0,
        index: 0,
        length: 2,
        proof: file.height() - 1,
        ..request
    };
    let response = crate::v2_wire::answer(leaf, &storage, &meta, &have).unwrap();
    assert_eq!(
        &response[53..85],
        digest(&data[..BLOCK as usize]).as_slice()
    );
    assert!(
        crate::v2_wire::answer(
            leaf,
            &storage,
            &meta,
            &crate::bitfield::Bitfield::new(meta.lengths.total_pieces())
        )
        .is_err()
    );
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    assert!(crate::v2_wire::answer_cancellable(leaf, &storage, &meta, &have, &cancel).is_err());
}

#[test]
fn v2_cache_revalidates_full_hash_and_piece_layers() {
    let meta = TorrentMeta::from_torrent_file(&sample(&vec![9; 100123], 32768)).unwrap();
    let folder = TempFolder::new();
    crate::resume::save_info(&folder.path(), &meta).unwrap();
    assert!(crate::resume::load_info(&folder.path(), meta.hashes, vec![]).is_some());
    let path = folder.path().join(format!("{}.layers", meta.hashes.key()));
    std::fs::write(path, b"de").unwrap();
    assert!(crate::resume::load_info(&folder.path(), meta.hashes, vec![]).is_none());
    let mut other = meta.hashes;
    other.v2.as_mut().unwrap().0[31] ^= 1;
    assert_eq!(other.wire(), meta.hashes.wire());
    assert_ne!(other.key(), meta.hashes.key());
    assert!(crate::resume::load_info(&folder.path(), other, vec![]).is_none());
}

#[test]
fn v2_full_magnets_and_hybrid_hashes_are_unambiguous() {
    let meta = TorrentMeta::from_torrent_file(&sample(&[1; 10], 16384)).unwrap();
    assert_eq!(
        InfoHashes::parse_magnet(&meta.hashes.magnet()).unwrap(),
        meta.hashes
    );
    assert_eq!(
        InfoHashes::parse_magnet(&meta.hashes.magnet().replace("urn:btmh:", "URN:BTMH:")).unwrap(),
        meta.hashes
    );
    let invalid = format!(
        "{}&xt=urn:btmh:1220{}",
        meta.hashes.magnet(),
        "ff".repeat(32)
    );
    assert!(InfoHashes::parse_magnet(&invalid).is_err());
    assert!(
        InfoHashes::parse_magnet(&format!("magnet:?xt=urn:btmh:1220{}", "a".repeat(63))).is_err()
    );
}

fn hybrid_sample(data: &[u8], piece: usize) -> Vec<u8> {
    use bencode::{BencodeValue as V, BencodeValueOwned};
    use buffers::ByteBufOwned;
    let mut value: BencodeValueOwned = bencode::from_bytes(&sample(data, piece)).unwrap();
    let V::Dict(top) = &mut value else {
        unreachable!()
    };
    let V::Dict(info) = top.get_mut(&ByteBufOwned::from(b"info".to_vec())).unwrap() else {
        unreachable!()
    };
    info.insert(
        ByteBufOwned::from(b"length".to_vec()),
        V::Integer(data.len() as i64),
    );
    info.insert(
        ByteBufOwned::from(b"pieces".to_vec()),
        V::Bytes(ByteBufOwned::from(crate::testkit::piece_hashes(
            data,
            piece as u32,
        ))),
    );
    let mut bytes = Vec::new();
    bencode::bencode_serialize_to_writer(&value, &mut bytes).unwrap();
    bytes
}

#[test]
fn hybrid_cache_preserves_aliases_and_btih_only_compatibility() {
    let meta = TorrentMeta::from_torrent_file(&hybrid_sample(&vec![3; 65539], 32768)).unwrap();
    let folder = TempFolder::new();
    crate::resume::save_info(&folder.path(), &meta).unwrap();
    let v1 = InfoHashes {
        v1: meta.hashes.v1,
        v2: None,
    };
    let v2 = InfoHashes {
        v1: None,
        v2: meta.hashes.v2,
    };
    assert!(
        crate::resume::load_info(&folder.path(), v1, vec![])
            .unwrap()
            .hashes
            .v2
            .is_none()
    );
    for identity in [v1, v2, meta.hashes] {
        assert_eq!(
            crate::cached_hashes(&folder.path(), identity),
            Some(meta.hashes)
        );
    }
    assert_eq!(
        TorrentMeta::from_hashes(v1, meta.info_bytes.clone(), vec![])
            .unwrap()
            .hashes,
        v1
    );
    let mut wrong = meta.hashes;
    wrong.v2.as_mut().unwrap().0[31] ^= 1;
    assert!(TorrentMeta::from_hashes(wrong, meta.info_bytes, vec![]).is_err());
}

#[test]
fn v2_resume_requires_sha256_version_and_rechecks_changed_files() {
    let data = vec![3; 65539];
    let meta = Arc::new(TorrentMeta::from_torrent_file(&sample(&data, 32768)).unwrap());
    let folder = TempFolder::new();
    let storage = Storage::new(meta.clone(), folder.path().join("out"));
    storage.write(0, &data).unwrap();
    storage.sync().unwrap();
    storage.close();
    let have = meta.wanted_pieces(None).unwrap();
    let state = folder.path().join("state");
    crate::resume::save(&state, &meta, &storage, &have).unwrap();
    assert_eq!(
        crate::resume::load(&state, &meta, &storage)
            .unwrap()
            .count(),
        have.count()
    );
    let path = state.join(format!("{}.resume.json", meta.hashes.key()));
    let original = std::fs::read(&path).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["version"] = 1.into();
    value.as_object_mut().unwrap().remove("v2_hash");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(crate::resume::load(&state, &meta, &storage).is_none());
    std::fs::write(&path, original).unwrap();
    std::fs::write(folder.path().join("out/sample.bin"), b"changed").unwrap();
    assert!(crate::resume::load(&state, &meta, &storage).is_none());
}

#[tokio::test]
async fn v2_hash_workers_are_bounded_and_release_on_cancellation() {
    let first = crate::v2_wire::hash_worker(0).await.unwrap();
    let second = crate::v2_wire::hash_worker(0).await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            crate::v2_wire::hash_worker(0)
        )
        .await
        .is_err()
    );
    drop(first);
    let third = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        crate::v2_wire::hash_worker(0),
    )
    .await
    .unwrap()
    .unwrap();
    drop((second, third));
}

#[test]
fn expensive_leaf_proof_is_refused_before_storage_access() {
    // A valid 4 GiB file with two 2 GiB pieces, described without allocating its
    // payload. The missing file proves rejection happens before any disk access.
    let hashes = [[1; 32], [2; 32]];
    let file_root = parent(hashes[0], hashes[1]);
    let info = d(vec![
        (
            b"file tree",
            d(vec![(
                b"sample.bin",
                d(vec![(
                    b"",
                    d(vec![
                        (b"length", i(4usize * 1024 * 1024 * 1024)),
                        (b"pieces root", b(&file_root)),
                    ]),
                )]),
            )]),
        ),
        (b"meta version", i(2)),
        (b"name", b(b"sample.bin")),
        (b"piece length", i(2usize * 1024 * 1024 * 1024)),
    ]);
    let metainfo = d(vec![
        (b"info", info),
        (b"piece layers", d(vec![(&file_root, b(&hashes.concat()))])),
    ]);
    let meta = Arc::new(TorrentMeta::from_torrent_file(&metainfo).unwrap());
    let folder = TempFolder::new();
    let storage = Storage::new(meta.clone(), folder.path());
    let request = Request {
        root: file_root,
        base: 0,
        index: 0,
        length: 2,
        proof: meta.v2[0].height() - 1,
    };
    let have = meta.wanted_pieces(None).unwrap();
    let error = crate::v2_wire::answer(request, &storage, &meta, &have).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Hash proof exceeds disk work limit"),
        "{error:#}"
    );
    // The same large torrent can still serve its authenticated piece layer
    // without touching the payload. Only the expensive lower-tree proof is denied.
    let layer = Request {
        base: meta.v2[0].layer,
        proof: 0,
        ..request
    };
    assert_eq!(layer.work_bytes(&meta).unwrap(), 0);
    let response = crate::v2_wire::answer(layer, &storage, &meta, &have).unwrap();
    assert_eq!(response.len(), layer.response_len() as usize);
    assert_eq!(layer.verify(&meta.v2[0], &response).unwrap(), hashes);
}

#[test]
fn hash_work_counts_siblings_short_blocks_and_padding_but_not_cached_layers() {
    let piece = 65536;
    let data = vec![5; piece * 2 + 17];
    let meta = TorrentMeta::from_torrent_file(&sample(&data, piece)).unwrap();
    let file = &meta.v2[0];
    let leaf = Request {
        root: file.root,
        base: 0,
        index: 0,
        length: 2,
        proof: file.height() - 1,
    };
    // The two requested leaves cost 32 KiB, their uncached sibling costs 32 KiB,
    // and the remaining proof siblings come from authenticated cached layers.
    assert_eq!(leaf.work_bytes(&meta).unwrap(), piece as u32);
    assert_eq!(Request { index: 8, ..leaf }.work_bytes(&meta).unwrap(), 17);
    assert_eq!(Request { index: 12, ..leaf }.work_bytes(&meta).unwrap(), 0);
    let layer = Request {
        base: file.layer,
        proof: file.height() - file.layer - 1,
        ..leaf
    };
    assert_eq!(layer.work_bytes(&meta).unwrap(), 0);
    for request in [
        Request { length: 0, ..leaf },
        Request {
            index: u32::MAX,
            ..leaf
        },
        Request { base: 32, ..leaf },
    ] {
        assert!(request.work_bytes(&meta).is_err());
    }
}
