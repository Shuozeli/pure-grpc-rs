# Codec Traits

**Source:** `tonic/src/codec/mod.rs` (160 lines), `tonic/src/codec/buffer.rs` (146 lines)
**Dependencies:** `bytes` (Buf, BufMut, BytesMut)

## Constants

```rust
const DEFAULT_CODEC_BUFFER_SIZE: usize = 8 * 1024;    // 8 KiB
const DEFAULT_YIELD_THRESHOLD: usize = 32 * 1024;     // 32 KiB
const DEFAULT_MAX_RECV_MESSAGE_SIZE: usize = 4 * 1024 * 1024;  // 4 MB
const DEFAULT_MAX_SEND_MESSAGE_SIZE: usize = usize::MAX;       // unlimited
pub const HEADER_SIZE: usize = 5;  // 1 byte compression + 4 bytes length
```

## Core Traits

### Codec

```rust
pub trait Codec {
    type Encode: Send + 'static;
    type Decode: Send + 'static;
    type Encoder: Encoder<Item = Self::Encode, Error = Status> + Send + 'static;
    type Decoder: Decoder<Item = Self::Decode, Error = Status> + Send + 'static;

    fn encoder(&mut self) -> Self::Encoder;
    fn decoder(&mut self) -> Self::Decoder;
}
```

The central pluggability point. `Codec` is a factory that produces `Encoder`
and `Decoder` instances. Parameterized by encode/decode message types.

Note: `Error` is pinned to `Status` on `Encoder`/`Decoder` associated types
in the `Codec` trait (not in the individual traits).

### Encoder

```rust
pub trait Encoder {
    type Item;
    type Error: From<io::Error>;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error>;
    fn buffer_settings(&self) -> BufferSettings { BufferSettings::default() }
}
```

Encodes a single message into the buffer. The framing (5-byte header) is
handled by the caller (encode.rs), not by the Encoder.

### Decoder

```rust
pub trait Decoder {
    type Item;
    type Error: From<io::Error>;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error>;
    fn buffer_settings(&self) -> BufferSettings { BufferSettings::default() }
}
```

Decodes a single message from the buffer. The buffer contains exactly the
bytes of one message (framing already stripped by decode.rs).

## BufferSettings

```rust
#[derive(Clone, Copy, Debug)]
pub struct BufferSettings {
    buffer_size: usize,       // initial capacity per RPC (default 8 KiB)
    yield_threshold: usize,   // batch encoding threshold (default 32 KiB)
}
```

- `buffer_size` — allocated per RPC, used as growth interval
- `yield_threshold` — in streaming, encoder batches messages until buffer
  exceeds this threshold before yielding a data frame

## Buffer Wrappers

### DecodeBuf

```rust
pub struct DecodeBuf<'a> {
    buf: &'a mut BytesMut,
    len: usize,    // exact number of bytes available for this message
}
```

Implements `Buf`. Limits reads to exactly `len` bytes (one message).
The underlying BytesMut may contain more data (next message).

- `remaining()` returns `self.len`
- `chunk()` returns at most `self.len` bytes
- `advance(cnt)` decrements `self.len`
- `copy_to_bytes(len)` decrements `self.len`, splits from BytesMut

### EncodeBuf

```rust
pub struct EncodeBuf<'a> {
    buf: &'a mut BytesMut,
}
```

Implements `BufMut` (unsafe impl). Thin wrapper that delegates everything to
BytesMut. Provides `reserve(additional)` for encoder use.

## Public Re-exports from codec module

```rust
pub use DecodeBuf, EncodeBuf;           // buffer.rs
pub use CompressionEncoding, EnabledCompressionEncodings;  // compression.rs
pub use Streaming;                       // decode.rs
pub use EncodeBody;                      // encode.rs
pub use SingleMessageCompressionOverride;  // compression.rs (doc-hidden)
```

## Notes for Our Implementation

1. **Trait design is clean** — adopt as-is. The `Error: From<io::Error>` bound
   on Encoder/Decoder (not `= Status`) enables non-Status error types that
   convert to Status.
2. **Codec forces `Error = Status`** — the more permissive Encoder/Decoder error
   types are narrowed at the Codec level.
3. **BufferSettings per-encoder/decoder** — each codec controls its own buffer
   allocation. Good for custom codecs (FlatBuffers might want larger buffers).
4. **Buffer wrappers are thin** — we should keep them. DecodeBuf's length-limiting
   is important for safety (prevents decoder from reading into next message).
5. **HEADER_SIZE = 5** — this is the gRPC framing constant, used everywhere.
