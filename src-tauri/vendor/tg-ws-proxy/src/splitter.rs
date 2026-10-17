//! MTProto message splitter.
//!
//! Each Telegram WebSocket message is expected to carry exactly **one**
//! complete MTProto transport packet. If raw TCP chunks are forwarded without
//! respecting packet boundaries, Telegram will reject or misparse the
//! connection.
//!
//! The upload bridge calls the splitter after decrypting the client stream and
//! before encrypting it for Telegram. Packet lengths can therefore be parsed
//! directly from the plaintext header. Only the encrypted packet currently
//! being assembled is retained; a completed packet is moved straight into its
//! WebSocket message without a second full-stream cipher or plaintext buffer.

use cipher::StreamCipher;

use crate::crypto::{AesCtr256, ProtoTag};

const MAX_HEADER_LEN: usize = 4;

pub struct MsgSplitter {
    proto: ProtoTag,
    packet: Vec<u8>,
    header: [u8; MAX_HEADER_LEN],
    header_len: usize,
    packet_len: Option<usize>,
    disabled: bool,
}

impl MsgSplitter {
    pub fn new(proto: ProtoTag) -> Self {
        Self {
            proto,
            packet: Vec::new(),
            header: [0; MAX_HEADER_LEN],
            header_len: 0,
            packet_len: None,
            disabled: false,
        }
    }

    /// Split plaintext MTProto bytes into packets and encrypt them in stream
    /// order for the Telegram WebSocket connection.
    pub fn split_and_encrypt(&mut self, plaintext: &mut [u8], enc: &mut AesCtr256) -> Vec<Vec<u8>> {
        if plaintext.is_empty() {
            return Vec::new();
        }

        if self.disabled {
            enc.apply_keystream(plaintext);
            return vec![plaintext.to_vec()];
        }

        let mut parts = Vec::new();
        let mut offset = 0;
        while offset < plaintext.len() {
            if self.packet_len.is_none() {
                let required = self.required_header_len();
                let take = (required - self.header_len).min(plaintext.len() - offset);
                let end = offset + take;
                self.header[self.header_len..self.header_len + take]
                    .copy_from_slice(&plaintext[offset..end]);
                self.header_len += take;
                self.encrypt_into_packet(&mut plaintext[offset..end], enc);
                offset = end;

                if self.header_len < required {
                    continue;
                }

                let Some(packet_len) = self.parsed_packet_len() else {
                    continue;
                };
                if packet_len == 0 {
                    self.encrypt_into_packet(&mut plaintext[offset..], enc);
                    parts.push(std::mem::take(&mut self.packet));
                    self.disabled = true;
                    self.header_len = 0;
                    return parts;
                }
                self.packet_len = Some(packet_len);
            }

            let packet_len = self.packet_len.expect("packet length was parsed");
            let remaining = packet_len - self.packet.len();
            let take = remaining.min(plaintext.len() - offset);
            let end = offset + take;
            self.encrypt_into_packet(&mut plaintext[offset..end], enc);
            offset = end;

            if self.packet.len() == packet_len {
                parts.push(std::mem::take(&mut self.packet));
                self.header_len = 0;
                self.packet_len = None;
            }
        }

        parts
    }

    /// Flush any encrypted bytes buffered for an incomplete final packet.
    pub fn flush(&mut self) -> Vec<Vec<u8>> {
        if self.packet.is_empty() {
            return Vec::new();
        }

        self.header_len = 0;
        self.packet_len = None;
        vec![std::mem::take(&mut self.packet)]
    }

    fn required_header_len(&self) -> usize {
        match self.proto {
            ProtoTag::Abridged
                if self.header_len == 0 || !matches!(self.header[0], 0x7f | 0xff) =>
            {
                1
            }
            ProtoTag::Abridged | ProtoTag::Intermediate | ProtoTag::PaddedIntermediate => 4,
        }
    }

    fn parsed_packet_len(&self) -> Option<usize> {
        match self.proto {
            ProtoTag::Abridged => {
                let extended = matches!(self.header[0], 0x7f | 0xff);
                let header_len = if extended { 4 } else { 1 };
                if self.header_len < header_len {
                    return None;
                }
                let words = if extended {
                    u32::from_le_bytes([self.header[1], self.header[2], self.header[3], 0]) as usize
                } else {
                    (self.header[0] & 0x7f) as usize
                };
                let payload_len = words.checked_mul(4)?;
                if payload_len == 0 {
                    Some(0)
                } else {
                    Some(payload_len.checked_add(header_len).unwrap_or(0))
                }
            }
            ProtoTag::Intermediate | ProtoTag::PaddedIntermediate => {
                if self.header_len < 4 {
                    return None;
                }
                let payload_len = (u32::from_le_bytes(self.header) & 0x7fff_ffff) as usize;
                if payload_len == 0 {
                    Some(0)
                } else {
                    Some(payload_len.checked_add(4).unwrap_or(0))
                }
            }
        }
    }

    fn encrypt_into_packet(&mut self, plaintext: &mut [u8], enc: &mut AesCtr256) {
        enc.apply_keystream(plaintext);
        self.packet.extend_from_slice(plaintext);
    }
}

#[cfg(test)]
mod tests;
