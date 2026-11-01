// Added by Download Engine: bound metainfo collections before allocation. See ../../patch.md.
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeSeed, Error, SeqAccess, Visitor},
};
use std::{fmt, marker::PhantomData};

pub const MAX_FILES: usize = 10_000;
pub const MAX_PATH_COMPONENTS: usize = 64;
pub const MAX_COMPONENT_BYTES: usize = 1024;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_TOTAL_PATH_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TRACKERS: usize = 32;
pub const MAX_TRACKER_URL_BYTES: usize = 2048;
pub const MAX_MAGNET_BYTES: usize = 128 * 1024;

struct RejectExtra;
impl<'de> DeserializeSeed<'de> for RejectExtra {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, _: D) -> Result<(), D::Error> {
        Err(D::Error::custom(
            "torrent collection exceeds the safety limit",
        ))
    }
}

struct Sequence<T> {
    limit: usize,
    marker: PhantomData<T>,
}

impl<T> Sequence<T> {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            marker: PhantomData,
        }
    }
}

impl<'de, T: Deserialize<'de>> Visitor<'de> for Sequence<T> {
    type Value = Vec<T>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a list with at most {} items", self.limit)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        // Never reserve a stranger's size_hint, or decode one extra unbounded element.
        let mut values = Vec::new();
        while values.len() < self.limit {
            match seq.next_element()? {
                Some(value) => values.push(value),
                None => return Ok(values),
            }
        }
        seq.next_element_seed(RejectExtra)?;
        Ok(values)
    }
}

impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Sequence<T> {
    type Value = Vec<T>;
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        de.deserialize_seq(self)
    }
}

struct OptionalSequence<T>(Sequence<T>);
impl<'de, T: Deserialize<'de>> Visitor<'de> for OptionalSequence<T> {
    type Value = Option<Vec<T>>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.expecting(f)
    }
    fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_some<D: Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        self.0.deserialize(de).map(Some)
    }
}

pub fn files<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    de: D,
) -> Result<Option<Vec<T>>, D::Error> {
    de.deserialize_option(OptionalSequence(Sequence::new(MAX_FILES)))
}

pub fn path<'de, D: Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Vec<T>, D::Error> {
    Sequence::new(MAX_PATH_COMPONENTS).deserialize(de)
}

pub fn optional_path<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    de: D,
) -> Result<Option<Vec<T>>, D::Error> {
    de.deserialize_option(OptionalSequence(Sequence::new(MAX_PATH_COMPONENTS)))
}

pub fn trackers<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    de: D,
) -> Result<Vec<Vec<T>>, D::Error> {
    struct Tiers<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Tiers<T> {
        type Value = Vec<Vec<T>>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a bounded tracker list")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut tiers = Vec::new();
            let mut remaining = MAX_TRACKERS;
            while tiers.len() < MAX_TRACKERS {
                match seq.next_element_seed(Sequence::new(remaining))? {
                    Some(tier) => {
                        remaining -= tier.len();
                        tiers.push(tier);
                    }
                    None => return Ok(tiers),
                }
            }
            seq.next_element_seed(RejectExtra)?;
            Ok(tiers)
        }
    }
    de.deserialize_seq(Tiers(PhantomData))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::torrent_metainfo::{TorrentMetaV1Info, torrent_from_bytes};
    use buffers::ByteBuf;

    fn info(files: usize, components: usize, component_bytes: usize) -> Vec<u8> {
        let component = format!("{component_bytes}:{}", "a".repeat(component_bytes));
        let file = format!("d6:lengthi1e4:pathl{}ee", component.repeat(components));
        format!(
            "d5:filesl{}e4:name1:x12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae",
            file.repeat(files)
        )
        .into_bytes()
    }

    #[test]
    fn file_and_component_limits_apply_during_bencode_decoding() {
        for (count, components, accepted) in [
            (MAX_FILES, 1, true),
            (MAX_FILES + 1, 1, false),
            (1, MAX_PATH_COMPONENTS, true),
            (1, MAX_PATH_COMPONENTS + 1, false),
        ] {
            let raw = info(count, components, 1);
            assert_eq!(
                bencode::from_bytes::<TorrentMetaV1Info<ByteBuf>>(&raw).is_ok(),
                accepted
            );
            let mut file = b"d4:info".to_vec();
            file.extend_from_slice(&raw);
            file.push(b'e');
            assert_eq!(torrent_from_bytes(&file).is_ok(), accepted);
        }
    }

    #[test]
    fn path_byte_and_total_name_limits_apply_before_building_paths() {
        for raw in [
            info(1, 1, MAX_COMPONENT_BYTES + 1),
            info(1, 5, MAX_COMPONENT_BYTES),
            info(
                MAX_TOTAL_PATH_BYTES / MAX_COMPONENT_BYTES + 1,
                1,
                MAX_COMPONENT_BYTES,
            ),
        ] {
            let decoded = bencode::from_bytes::<TorrentMetaV1Info<ByteBuf>>(&raw).unwrap();
            assert!(matches!(
                decoded.validate(),
                Err(crate::Error::BadTorrentComplexity)
            ));
        }
    }

    #[test]
    fn tracker_limits_count_all_tiers_and_empty_tiers() {
        let raw_info = info(1, 1, 1);
        for (list, accepted) in [
            (format!("l{}e", "1:a".repeat(MAX_TRACKERS)), true),
            (format!("l{}e", "1:a".repeat(MAX_TRACKERS + 1)), false),
            ("l1:ae".repeat(MAX_TRACKERS + 1), false),
            ("le".repeat(MAX_TRACKERS + 1), false),
        ] {
            let mut file = format!("d13:announce-listl{list}e4:info").into_bytes();
            file.extend_from_slice(&raw_info);
            file.push(b'e');
            assert_eq!(torrent_from_bytes(&file).is_ok(), accepted);
        }
    }
}
