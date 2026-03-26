//! Shared test codec for `Vec<u8>` messages.
//!
//! Used across workspace crates to avoid duplicating the same test codec.

use super::{BufferSettings, Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use crate::Status;
use bytes::{Buf, BufMut};

/// A codec that encodes/decodes raw `Vec<u8>` messages with no framing.
#[derive(Debug, Default, Clone)]
pub struct TestCodec;

/// Encoder half of [`TestCodec`].
#[derive(Debug, Default)]
pub struct TestEncoder;

/// Decoder half of [`TestCodec`].
#[derive(Debug, Default)]
pub struct TestDecoder;

impl Codec for TestCodec {
    type Encode = Vec<u8>;
    type Decode = Vec<u8>;
    type Encoder = TestEncoder;
    type Decoder = TestDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        TestEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        TestDecoder
    }
}

impl Encoder for TestEncoder {
    type Item = Vec<u8>;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, buf: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        buf.put_slice(&item);
        Ok(())
    }
}

impl Decoder for TestDecoder {
    type Item = Vec<u8>;
    type Error = Status;

    fn decode(&mut self, buf: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let len = buf.remaining();
        if len == 0 {
            return Ok(None);
        }
        let data = buf.copy_to_bytes(len).to_vec();
        Ok(Some(data))
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}
