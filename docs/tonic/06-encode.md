# Encode (Framing)

**Source:** `tonic/src/codec/encode.rs` (335 lines)
**Dependencies:** `bytes`, `http`, `http-body`, `pin-project`, `tokio-stream`

## gRPC Wire Format

```
[1 byte: compression flag (0=no, 1=yes)]
[4 bytes: u32 big-endian message length]
[N bytes: message payload]
```

## Key Types

### EncodedBytes<T, U> (private Stream)

```rust
struct EncodedBytes<T, U> {
    source: Fuse<U>,                           // upstream message stream (fused)
    encoder: T,                                // the Encoder impl
    compression_encoding: Option<CompressionEncoding>,
    max_message_size: Option<usize>,
    buf: BytesMut,                             // accumulation buffer
    uncompression_buf: BytesMut,               // temp buffer for compression
    error: Option<Status>,                     // deferred error
}
```

`Stream<Item = Result<Bytes, Status>>` — encodes messages into framed bytes.

### EncodeBody<T, U> (public Body)

```rust
pub struct EncodeBody<T, U> {
    inner: EncodedBytes<T, U>,
    state: EncodeState,
}

struct EncodeState {
    error: Option<Status>,
    role: Role,           // Client or Server
    is_end_stream: bool,
}
```

Wraps `EncodedBytes` as an `http_body::Body`. The critical difference
between client and server is **trailer handling**.

## Encoding Flow

### `encode_item()` — Single Message

1. Record current buffer offset
2. **Reserve 5 bytes** for header (advance unsafely)
3. If compression enabled:
   - Encode message into `uncompression_buf`
   - Compress from `uncompression_buf` into `buf`
4. If no compression:
   - Encode message directly into `buf`
5. **Backfill header**: write compression flag (u8) and length (u32 BE) at offset

### `finish_encoding()` — Header Backfill

```rust
fn finish_encoding(compression, max_message_size, buf: &mut [u8]) -> Result<(), Status>
```

1. Calculate `len = buf.len() - HEADER_SIZE`
2. Check against `max_message_size` -> `OutOfRange` error
3. Check against `u32::MAX` -> `ResourceExhausted` error
4. Write: `[compression_flag as u8][len as u32 BE]`

### `EncodedBytes::poll_next()` — Stream Batching

```
loop {
    poll source:
      Pending + buf empty    -> return Pending
      None    + buf empty    -> return None (stream done)
      Pending | None + buf   -> yield buf.split().freeze()  (flush batch)
      Ok(item)               -> encode_item(); if buf >= yield_threshold, yield
      Err(status) + buf      -> save error, yield buf first
      Err(status) + no buf   -> return Err immediately
}
```

**Batching optimization**: messages keep accumulating in `buf` until either:
- Source is not ready (Pending)
- Source is exhausted (None)
- Buffer exceeds `yield_threshold` (default 32 KiB)

This batches small messages into larger HTTP/2 DATA frames.

## EncodeBody — Client vs Server

### Client (`new_client`)
- No compression override
- On error: returns `Err(status)` directly
- No trailers on stream end

### Server (`new_server`)
- Accepts `SingleMessageCompressionOverride`
- On error: emits `Frame::trailers(status.to_header_map()?)` with error status
- On stream end: emits `Frame::trailers(Status::ok("").to_header_map()?)`
- Sets `is_end_stream = true` after trailers

**This is a key gRPC protocol requirement**: server responses MUST end with
trailing headers containing `grpc-status` (and optionally `grpc-message`).

## Notes for Our Implementation

1. **Reserve-then-backfill pattern** is essential — reserve 5 bytes, encode message,
   then write the header knowing the actual length
2. **Batching via yield_threshold** is important for performance — don't yield
   a frame per message in streaming RPCs
3. **Server trailers are mandatory** — the EncodeBody must append `grpc-status`
   trailers even on success (Status::Ok)
4. **Client never sends trailers** — client body is just data frames
5. **Error deferral**: if error occurs with buffered data, yield data first,
   save error for next poll. This ensures partial data isn't lost.
6. We can skip compression initially — just handle the uncompressed path
