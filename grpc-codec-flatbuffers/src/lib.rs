//! FlatBuffers codec for the pure-grpc-rs framework.
//!
//! Provides [`FlatBuffersCodec`] which implements `grpc_core::codec::Codec`
//! for FlatBuffers message types.
//!
//! # Message Trait
//!
//! Types used with this codec must implement [`FlatBufferGrpcMessage`],
//! which bridges between owned types (for gRPC's `Send + 'static` requirement)
//! and FlatBuffers' zero-copy reader types.
//!
//! Generated code (from flatbuffers-rs Object API) produces owned `XxxT`
//! structs that implement `pack()` and `unpack()`.

use bytes::{Buf, BufMut};
use grpc_core::codec::{BufferSettings, Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use grpc_core::Status;
use std::marker::PhantomData;

/// Trait that FlatBuffers gRPC message types must implement.
///
/// This bridges the owned type (used in gRPC handlers) with FlatBuffers
/// serialization/deserialization.
pub trait FlatBufferGrpcMessage: Send + Sync + Clone + 'static {
    /// Encode this message into a FlatBuffer, returning the raw bytes.
    fn encode_flatbuffer(&self) -> Vec<u8>;

    /// Decode a message from raw FlatBuffer bytes.
    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String>;
}

/// A [`Codec`] for FlatBuffers messages.
#[derive(Debug, Clone)]
pub struct FlatBuffersCodec<T, U> {
    _pd: PhantomData<(T, U)>,
}

impl<T, U> FlatBuffersCodec<T, U> {
    pub fn new() -> Self {
        Self { _pd: PhantomData }
    }
}

impl<T, U> Default for FlatBuffersCodec<T, U> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, U> Codec for FlatBuffersCodec<T, U>
where
    T: FlatBufferGrpcMessage,
    U: FlatBufferGrpcMessage,
{
    type Encode = T;
    type Decode = U;
    type Encoder = FlatBuffersEncoder<T>;
    type Decoder = FlatBuffersDecoder<U>;

    fn encoder(&mut self) -> Self::Encoder {
        FlatBuffersEncoder { _pd: PhantomData }
    }

    fn decoder(&mut self) -> Self::Decoder {
        FlatBuffersDecoder { _pd: PhantomData }
    }
}

/// Encoder for FlatBuffers messages.
#[derive(Debug, Clone, Default)]
pub struct FlatBuffersEncoder<T> {
    _pd: PhantomData<T>,
}

impl<T: FlatBufferGrpcMessage> Encoder for FlatBuffersEncoder<T> {
    type Item = T;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, buf: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        let data = item.encode_flatbuffer();
        buf.reserve(data.len());
        buf.put_slice(&data);
        Ok(())
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}

/// Decoder for FlatBuffers messages.
#[derive(Debug, Clone, Default)]
pub struct FlatBuffersDecoder<U> {
    _pd: PhantomData<U>,
}

impl<U: FlatBufferGrpcMessage> Decoder for FlatBuffersDecoder<U> {
    type Item = U;
    type Error = Status;

    fn decode(&mut self, buf: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let len = buf.remaining();
        let data = buf.copy_to_bytes(len);
        let item = U::decode_flatbuffer(&data)
            .map_err(|e| Status::internal(format!("FlatBuffers decode error: {e}")))?;
        Ok(Some(item))
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}

// Re-export for convenience
pub use flatbuffers;

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use grpc_core::codec::{DecodeBuf, EncodeBuf};

    // Simple test type
    #[derive(Debug, Clone, PartialEq)]
    struct TestMessage {
        value: String,
    }

    impl FlatBufferGrpcMessage for TestMessage {
        fn encode_flatbuffer(&self) -> Vec<u8> {
            // Simple encoding: just the raw string bytes for testing
            self.value.as_bytes().to_vec()
        }

        fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
            let value =
                String::from_utf8(data.to_vec()).map_err(|e| format!("invalid utf8: {e}"))?;
            Ok(TestMessage { value })
        }
    }

    #[test]
    fn codec_produces_encoder_decoder() {
        let mut codec = FlatBuffersCodec::<TestMessage, TestMessage>::default();
        let _encoder = codec.encoder();
        let _decoder = codec.decoder();
    }

    #[test]
    fn encode_decode_roundtrip() {
        let mut encoder = FlatBuffersEncoder::<TestMessage> { _pd: PhantomData };
        let mut decoder = FlatBuffersDecoder::<TestMessage> { _pd: PhantomData };

        let msg = TestMessage {
            value: "hello flatbuffers".into(),
        };

        // Encode
        let mut bytes = BytesMut::new();
        let mut buf = EncodeBuf::new(&mut bytes);
        encoder.encode(msg.clone(), &mut buf).unwrap();

        // Decode
        let len = bytes.len();
        let mut decode_buf = DecodeBuf::new(&mut bytes, len);
        let decoded = decoder.decode(&mut decode_buf).unwrap().unwrap();

        assert_eq!(decoded, msg);
    }
}
