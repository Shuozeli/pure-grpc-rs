use super::{BufferSettings, Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use crate::Status;
use bytes::Buf;
use prost::Message;
use std::marker::PhantomData;

/// A [`Codec`] that implements `application/grpc+proto` via the prost library.
#[derive(Debug, Clone)]
pub struct ProstCodec<T, U> {
    _pd: PhantomData<(T, U)>,
}

impl<T, U> ProstCodec<T, U> {
    pub fn new() -> Self {
        Self { _pd: PhantomData }
    }
}

impl<T, U> Default for ProstCodec<T, U> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, U> Codec for ProstCodec<T, U>
where
    T: Message + Send + 'static,
    U: Message + Default + Send + 'static,
{
    type Encode = T;
    type Decode = U;
    type Encoder = ProstEncoder<T>;
    type Decoder = ProstDecoder<U>;

    fn encoder(&mut self) -> Self::Encoder {
        ProstEncoder {
            _pd: PhantomData,
            buffer_settings: BufferSettings::default(),
        }
    }

    fn decoder(&mut self) -> Self::Decoder {
        ProstDecoder {
            _pd: PhantomData,
            buffer_settings: BufferSettings::default(),
        }
    }
}

/// Encodes prost messages.
#[derive(Debug, Clone, Default)]
pub struct ProstEncoder<T> {
    _pd: PhantomData<T>,
    buffer_settings: BufferSettings,
}

impl<T: Message> Encoder for ProstEncoder<T> {
    type Item = T;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, buf: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(buf)
            .expect("Message only errors if not enough space");
        Ok(())
    }

    fn buffer_settings(&self) -> BufferSettings {
        self.buffer_settings
    }
}

/// Decodes prost messages.
#[derive(Debug, Clone, Default)]
pub struct ProstDecoder<U> {
    _pd: PhantomData<U>,
    buffer_settings: BufferSettings,
}

impl<U: Message + Default> Decoder for ProstDecoder<U> {
    type Item = U;
    type Error = Status;

    fn decode(&mut self, buf: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let item = Message::decode(buf as &mut dyn Buf)
            .map(Some)
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(item)
    }

    fn buffer_settings(&self) -> BufferSettings {
        self.buffer_settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    // Use a simple prost message for testing: raw bytes wrapper
    // We test with prost's built-in encoding for a small struct

    #[test]
    fn prost_encoder_writes_bytes() {
        // Encode an empty message (prost encodes nothing for default values)
        let mut encoder = ProstEncoder::<()>::default();

        let mut bytes = BytesMut::with_capacity(64);
        let mut buf = EncodeBuf::new(&mut bytes);
        encoder.encode((), &mut buf).unwrap();

        // Empty prost message produces 0 bytes
        assert_eq!(bytes.len(), 0);
    }

    #[test]
    fn prost_decoder_decodes_empty_message() {
        let mut decoder = ProstDecoder::<()>::default();

        let mut bytes = BytesMut::new();
        let mut buf = DecodeBuf::new(&mut bytes, 0);
        let result = decoder.decode(&mut buf).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn prost_codec_produces_encoder_decoder() {
        let mut codec = ProstCodec::<(), ()>::default();
        let _encoder = codec.encoder();
        let _decoder = codec.decoder();
    }
}
