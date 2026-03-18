# Decode (Streaming)

**Source:** `tonic/src/codec/decode.rs` (431 lines)
**Dependencies:** `bytes`, `http`, `http-body`, `http-body-util`, `sync-wrapper`, `tracing`

## Overview

`Streaming<T>` is the decoder side — it reads HTTP/2 body frames, parses
the gRPC framing, and yields decoded messages as a `Stream<Item = Result<T, Status>>`.

## Type Definitions

```rust
pub struct Streaming<T> {
    decoder: SyncWrapper<Box<dyn Decoder<Item = T, Error = Status> + Send + 'static>>,
    inner: StreamingInner,
}

struct StreamingInner {
    body: SyncWrapper<Body>,
    state: State,
    direction: Direction,
    buf: BytesMut,                             // accumulation buffer
    trailers: Option<HeaderMap>,               // stored trailers
    decompress_buf: BytesMut,                  // decompression temp buffer
    encoding: Option<CompressionEncoding>,      // expected compression
    max_message_size: Option<usize>,
}
```

### State Machine

```rust
enum State {
    ReadHeader,                    // waiting for 5-byte frame header
    ReadBody {
        compression: Option<CompressionEncoding>,
        len: usize,               // expected body length
    },
    Error(Option<Status>),         // terminal error state (yield once, then None)
}
```

### Direction

```rust
enum Direction {
    Request,                       // decoding a request body (server side)
    Response(StatusCode),          // decoding a response body (client side)
    EmptyResponse,                 // headers-only response (no body expected)
}
```

Direction affects error handling:
- Request + Cancelled = gracefully end stream (client disconnected)
- Response = check trailers for grpc-status on stream end

## Constructors

```rust
// Client-side (decoding server response)
Streaming::new_response(decoder, body, status_code, encoding, max_message_size)

// Server-side (decoding client request)
Streaming::new_request(decoder, body, encoding, max_message_size)

// Empty response (trailers-only)
Streaming::new_empty(decoder, body)
```

All delegate to private `new()` which:
1. Type-erases decoder into `Box<dyn Decoder>`
2. Wraps body in `Body::new()` with error mapping
3. Maps body frames to `Bytes` (via `copy_to_bytes`)
4. Initializes state to `ReadHeader`

## Decoding Flow

### `decode_chunk()` — Frame Parsing

**ReadHeader state:**
1. Need at least 5 bytes in `buf`
2. Read compression flag (u8): 0=none, 1=compressed
   - Flag=1 but no encoding set -> `INTERNAL` error
   - Flag>1 -> `INTERNAL` error
3. Read length (u32 BE)
4. Check against `max_message_size` -> `OUT_OF_RANGE` error
5. Reserve `len` bytes, transition to `ReadBody { compression, len }`

**ReadBody state:**
1. Need at least `len` bytes in `buf`
2. If compressed: decompress into `decompress_buf`, create DecodeBuf from that
3. If not compressed: create DecodeBuf directly from `buf`
4. Return the DecodeBuf for decoding

### `poll_frame()` — Body Polling

Polls `body.poll_frame()`:
- **Data frame**: append to `buf`, return `Some(())`
- **Trailers frame**: store in `self.trailers`, return `None`
- **Error**: if Request + Cancelled, graceful end; otherwise transition to Error state
- **EOF**: if `buf` has remaining data, `INTERNAL` error (unexpected EOF)

### `response()` — End-of-Stream Status Check

Only for `Direction::Response`:
1. Calls `infer_grpc_status(trailers, status_code)`
2. If grpc-status in trailers is non-OK, returns error
3. If trailers missing grpc-status, maps from HTTP status code

### `Stream::poll_next()` — Main Loop

```
loop {
    if state == Error: yield error once, then None
    if decode_chunk() has item: yield Ok(item)
    poll_frame():
      Some(()) -> loop again (got data, try decode)
      None     -> response() check, then None or Error
}
```

## Public API

```rust
// Async convenience method
pub async fn message(&mut self) -> Result<Option<T>, Status>

// Drain stream and return trailing metadata
pub async fn trailers(&mut self) -> Result<Option<MetadataMap>, Status>
```

`trailers()` first checks cached trailers, then drains all messages to
trigger trailer reception, then checks again.

## SyncWrapper Usage

`SyncWrapper` makes `Streaming<T>: Sync` even though `Body` and
`dyn Decoder` are not `Sync`. This is safe because `Streaming` is `Unpin`
and only accessed via `&mut self`.

```rust
impl<T> Unpin for Streaming<T> {}
static_assertions::assert_impl_all!(Streaming<()>: Send, Sync);
```

## Notes for Our Implementation

1. **State machine is straightforward** — ReadHeader -> ReadBody -> ReadHeader cycle
2. **Direction matters** — server decoding requests vs client decoding responses
   have different error handling (Cancelled = graceful for requests)
3. **Trailers are critical** — gRPC status is in trailers, must be checked on
   stream end. `infer_grpc_status` is the key function.
4. **SyncWrapper trick** — needed because tokio tasks require `Send + Sync` for
   the response body. Consider using `sync_wrapper` crate or manual impl.
5. **Box<dyn Decoder>** — type-erases the decoder. Cost is one allocation per
   stream, acceptable.
6. Skip compression initially — just handle uncompressed path in decode_chunk.
7. **Unexpected EOF** — if body ends with partial data in buf, return INTERNAL error.
