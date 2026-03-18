# ProstCodec

**Source:** `tonic-prost/src/codec.rs` (406 lines, 260 lines of tests)
**Dependencies:** `prost`, `tonic` (codec traits)
**Crate:** `tonic-prost` (separate crate, 31-line lib.rs)

## Overview

Trivially thin codec that delegates to `prost::Message`. This is the
reference implementation showing how simple a Codec can be.

## Type Definitions

```rust
#[derive(Debug, Clone)]
pub struct ProstCodec<T, U> {
    _pd: PhantomData<(T, U)>,    // T=encode type, U=decode type
}

#[derive(Debug, Clone, Default)]
pub struct ProstEncoder<T> {
    _pd: PhantomData<T>,
    buffer_settings: BufferSettings,
}

#[derive(Debug, Clone, Default)]
pub struct ProstDecoder<U> {
    _pd: PhantomData<U>,
    buffer_settings: BufferSettings,
}
```

All three types are zero-sized (besides buffer_settings). No state needed.

## Codec Implementation

```rust
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
        ProstEncoder { _pd: PhantomData, buffer_settings: BufferSettings::default() }
    }
    fn decoder(&mut self) -> Self::Decoder {
        ProstDecoder { _pd: PhantomData, buffer_settings: BufferSettings::default() }
    }
}
```

Note: `U: Default` is required by prost for decoding (creates default then
merges).

## Encoder

```rust
impl<T: Message> Encoder for ProstEncoder<T> {
    type Item = T;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, buf: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(buf).expect("Message only errors if not enough space");
        Ok(())
    }
}
```

Calls `prost::Message::encode(buf)` which writes protobuf wire format
directly into the EncodeBuf. The `expect` is safe because EncodeBuf wraps
a BytesMut which auto-grows.

## Decoder

```rust
impl<U: Message + Default> Decoder for ProstDecoder<U> {
    type Item = U;
    type Error = Status;

    fn decode(&mut self, buf: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let item = Message::decode(buf)
            .map(Option::Some)
            .map_err(from_decode_error)?;
        Ok(item)
    }
}

fn from_decode_error(error: prost::DecodeError) -> Status {
    Status::internal(error.to_string())
}
```

Calls `prost::Message::decode(buf)` which reads from the DecodeBuf (implements
`Buf`). Decode errors are mapped to `Status::internal` per gRPC spec.

## Custom Buffer Settings

```rust
ProstCodec::<T, U>::raw_encoder(buffer_settings) -> ProstEncoder<T>
ProstCodec::<T, U>::raw_decoder(buffer_settings) -> ProstDecoder<U>
```

Factory methods for custom buffer settings (used in codec_buffers example).

## Crate Structure

```
tonic-prost/
  src/lib.rs    - re-exports ProstCodec, ProstEncoder, ProstDecoder, prost
  src/codec.rs  - all implementation + tests
```

Separate crate because prost is an optional dependency. Users who want a
different serialization format don't need to pull in prost at all.

## Notes for Our Implementation

1. **Our ProstCodec goes in grpc-core behind feature flag** — we put it in
   `grpc-core/src/codec/prost.rs` gated on `feature = "prost-codec"` since
   we already have our own `protobuf-rs` compiler. Alternatively, it could
   be a separate crate like tonic does.
2. **Encoder is infallible** — prost encode only fails if buffer too small,
   but BytesMut grows. The `expect` is correct.
3. **Decoder maps errors to INTERNAL** — per gRPC spec, protobuf parse errors
   are INTERNAL status.
4. **Zero-sized codec** — no state needed, just PhantomData. Very cheap to
   create per-RPC.
5. **For our protobuf-rs integration**, we'll need a similar codec where
   `T: protobuf_rs::Message` instead of `T: prost::Message`.
