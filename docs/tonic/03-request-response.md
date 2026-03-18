# Request & Response

**Source:** `tonic/src/request.rs` (504 lines), `tonic/src/response.rs` (155 lines)
**Dependencies:** `http` (Extensions, Uri, Method, Version), metadata, tokio-stream

## Core Types

```rust
#[derive(Debug)]
pub struct Request<T> {
    metadata: MetadataMap,
    message: T,
    extensions: Extensions,  // http::Extensions
}

#[derive(Debug)]
pub struct Response<T> {
    metadata: MetadataMap,
    message: T,
    extensions: Extensions,
}
```

Both are simple three-field wrappers. `T` is the message type — can be a
single message (unary) or a `Streaming<M>` (streaming).

## Request API

### Construction
- `new(message)` — empty metadata/extensions
- `from_parts(MetadataMap, Extensions, T)` — full construction
- `from_http(http::Request<T>)` — convert from HTTP, headers become metadata
- `from_http_parts(Parts, T)` — internal, used by server

### Accessors
- `get_ref() / get_mut()` — message access
- `metadata() / metadata_mut()` — metadata access
- `extensions() / extensions_mut()` — http::Extensions
- `into_inner()` — consume, return message
- `into_parts()` — `(MetadataMap, Extensions, T)`
- `map(f)` — transform message, keep metadata/extensions

### Server-side (feature-gated)
- `local_addr()` — from TcpConnectInfo extension
- `remote_addr()` — from TcpConnectInfo extension
- `peer_certs()` — from TlsConnectInfo extension (mTLS)

### Timeout
```rust
pub fn set_timeout(&mut self, deadline: Duration)
```
Encodes duration into `grpc-timeout` header per gRPC spec.

### IntoRequest Trait
```rust
pub trait IntoRequest<T>: sealed::Sealed {
    fn into_request(self) -> Request<T>;
}

// Blanket impls:
impl<T> IntoRequest<T> for T           // T -> Request::new(T)
impl<T> IntoRequest<T> for Request<T>  // passthrough
```

Enables `client.method(msg)` and `client.method(Request::new(msg))` interchangeably.

### IntoStreamingRequest Trait
```rust
pub trait IntoStreamingRequest: sealed::Sealed {
    type Stream: Stream<Item = Self::Message> + Send + 'static;
    type Message;
    fn into_streaming_request(self) -> Request<Self::Stream>;
}

// Blanket impls:
impl<T: Stream + Send + 'static> IntoStreamingRequest for T
impl<T: Stream + Send + 'static> IntoStreamingRequest for Request<T>
```

## Response API

Almost identical to Request but simpler:
- `new(message)` / `from_parts(metadata, message, extensions)`
- `get_ref() / get_mut() / metadata() / metadata_mut()`
- `into_inner() / into_parts()` — note: parts order is `(MetadataMap, T, Extensions)`
- `map(f)` / `extensions() / extensions_mut()`
- `from_http(http::Response<T>)` / `into_http()`
- `disable_compression()` — inserts compression override extension
- `From<T>` impl — `T` automatically wraps into `Response::new(T)`

### into_http Differences
- Request: takes uri/method/version/sanitize_headers params
- Response: always HTTP/2, always sanitizes headers

## gRPC Timeout Encoding

```rust
fn duration_to_grpc_timeout(duration: Duration) -> String
```

Per [gRPC spec](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md):
- Format: `{value}{unit}` where value is at most 8 decimal digits
- Units tried in order (most precise first): `n`(ns), `u`(us), `m`(ms), `S`, `M`, `H`
- Example: 30s -> `"30000000u"` (microseconds, fits in 8 digits)

## SanitizeHeaders

```rust
pub(crate) enum SanitizeHeaders { Yes, No }
```

Controls whether gRPC-reserved headers (te, content-type, grpc-message,
grpc-message-type, grpc-status) are stripped when converting to HTTP.
Client uses `No` (it sets its own), server uses `Yes` (strips user-set reserved headers).

## Notes for Our Implementation

1. **Simple wrappers** — these are straightforward, implement early
2. **`IntoRequest` blanket impl** is important for ergonomic client APIs
3. **Timeout encoding** must follow gRPC spec exactly (8-digit max, unit suffix)
4. **Extensions** are used throughout for passing connection info, compression
   overrides, and GrpcMethod markers — keep `http::Extensions`
5. **Response `into_parts` order differs from Request** — `(metadata, message, extensions)`
   vs Request's `(metadata, extensions, message)`. We should pick one consistent order.
