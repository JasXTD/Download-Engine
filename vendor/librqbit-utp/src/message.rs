use tracing::trace;

use crate::{
    Payload,
    raw::{Type, UtpHeader},
};

#[derive(Default, Clone, PartialEq, Eq)]
pub struct UtpMessage {
    pub header: UtpHeader,
    pub data: Payload,
}

impl std::fmt::Debug for UtpMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.header.short_repr())?;
        if cfg!(test) {
            write!(
                f,
                ":payload_len={}:payload={:?}",
                self.payload().len(),
                String::from_utf8_lossy(self.payload())
            )
        } else {
            write!(f, ":payload_len={}", self.payload().len())
        }
    }
}

impl UtpMessage {
    #[cfg(test)]
    pub fn new_test(header: UtpHeader, payload: &[u8]) -> Self {
        UtpMessage {
            header,
            data: payload.to_owned(),
        }
    }

    pub fn deserialize(buf: &[u8]) -> Option<Self> {
        let (header, hsize) = UtpHeader::deserialize(buf)?;

        let payload_size = buf.len() - hsize;

        match header.get_type() {
            Type::ST_DATA => {
                if payload_size == 0 {
                    trace!("ST_DATA packet with 0 payload, ignoring");
                    return None;
                }
            }
            other => {
                if payload_size > 0 {
                    trace!("{other:?} packet with payload, ignoring");
                    return None;
                }
            }
        }

        Some(Self {
            header,
            data: buf[hsize..].to_owned(),
        })
    }

    pub fn payload(&self) -> &[u8] {
        &self.data
    }
}

#[cfg(test)]
mod malformed_tests {
    use super::*;

    #[test]
    fn arbitrary_truncated_headers_extensions_and_payloads_do_not_panic() {
        let mut state = 0x12345678u32;
        for len in 0..=2048 {
            let mut packet = vec![0; len];
            for byte in &mut packet {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *byte = state as u8;
            }
            for kind in [0x01, 0x11, 0x21, 0x31, 0x41, 0xf1, 0x02] {
                if let Some(first) = packet.first_mut() {
                    *first = kind;
                }
                let _ = UtpMessage::deserialize(&packet);
            }
        }
        let mut truncated = vec![0; 21];
        truncated[0] = 0x21;
        truncated[1] = 1;
        assert!(UtpMessage::deserialize(&truncated).is_none());
        truncated.push(255);
        assert!(UtpMessage::deserialize(&truncated).is_none());
    }
}
