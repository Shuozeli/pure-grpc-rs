mod buffer;
pub mod compression;
mod decode;
mod encode;

#[cfg(feature = "prost-codec")]
pub mod prost_codec;

#[cfg(any(test, feature = "test-helpers"))]
pub mod test_helpers;

use crate::Status;
use std::io;

pub use self::buffer::{DecodeBuf, EncodeBuf};
pub use self::compression::{CompressionEncoding, EnabledCompressionEncodings};
pub use self::decode::Streaming;
pub use self::encode::EncodeBody;

/// gRPC frame header size: 1 byte compression flag + 4 bytes length.
pub const HEADER_SIZE: usize = std::mem::size_of::<u8>() + std::mem::size_of::<u32>();

const DEFAULT_CODEC_BUFFER_SIZE: usize = 8 * 1024;
const DEFAULT_YIELD_THRESHOLD: usize = 32 * 1024;
const DEFAULT_MAX_RECV_MESSAGE_SIZE: usize = 4 * 1024 * 1024;
const DEFAULT_MAX_SEND_MESSAGE_SIZE: usize = usize::MAX;

/// Settings for buffer allocation and growth.
#[derive(Clone, Copy, Debug)]
pub struct BufferSettings {
    pub buffer_size: usize,
    pub yield_threshold: usize,
}

impl BufferSettings {
    pub fn new(buffer_size: usize, yield_threshold: usize) -> Self {
        Self {
            buffer_size,
            yield_threshold,
        }
    }
}

impl Default for BufferSettings {
    fn default() -> Self {
        Self {
            buffer_size: DEFAULT_CODEC_BUFFER_SIZE,
            yield_threshold: DEFAULT_YIELD_THRESHOLD,
        }
    }
}

/// Trait that knows how to encode and decode gRPC messages.
pub trait Codec {
    type Encode: Send + 'static;
    type Decode: Send + 'static;
    type Encoder: Encoder<Item = Self::Encode, Error = Status> + Send + 'static;
    type Decoder: Decoder<Item = Self::Decode, Error = Status> + Send + 'static;

    fn encoder(&mut self) -> Self::Encoder;
    fn decoder(&mut self) -> Self::Decoder;
}

/// Encodes gRPC message types.
pub trait Encoder {
    type Item;
    type Error: From<io::Error>;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error>;

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}

/// Decodes gRPC message types.
pub trait Decoder {
    type Item;
    type Error: From<io::Error>;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error>;

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}
