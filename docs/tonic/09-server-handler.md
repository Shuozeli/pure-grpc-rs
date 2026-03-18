# Server Handler

**Source:** `tonic/src/server/grpc.rs` (497 lines)
**Dependencies:** `http-body`, `tokio-stream`

## Overview

`Grpc<T: Codec>` is the server-side handler that dispatches incoming HTTP/2
requests to the appropriate service method. It handles all 4 gRPC patterns.

## Type Definition

```rust
pub struct Grpc<T> {
    codec: T,
    accept_compression_encodings: EnabledCompressionEncodings,
    send_compression_encodings: EnabledCompressionEncodings,
    max_decoding_message_size: Option<usize>,
    max_encoding_message_size: Option<usize>,
}
```

## Builder Methods

```rust
Grpc::new(codec) -> Self
    .accept_compressed(encoding) -> Self
    .send_compressed(encoding) -> Self
    .max_decoding_message_size(limit) -> Self
    .max_encoding_message_size(limit) -> Self
    .apply_compression_config(accept, send) -> Self      // doc-hidden, used by codegen
    .apply_max_message_size_config(dec, enc) -> Self      // doc-hidden, used by codegen
```

## Four Handler Methods

### Unary

```rust
pub async fn unary<S, B>(&mut self, service: S, req: http::Request<B>) -> http::Response<Body>
where S: UnaryService<T::Decode, Response = T::Encode>
```

1. Extract `accept_encoding` from request headers
2. `map_request_unary`: decode single message from body
3. Call `service.call(request)` -> `Result<Response<T::Encode>, Status>`
4. Wrap response message in `tokio_stream::once(Ok(m))`
5. `map_response`: encode stream into `EncodeBody`, return HTTP response

### Server Streaming

```rust
pub async fn server_streaming<S, B>(&mut self, service: S, req: http::Request<B>) -> http::Response<Body>
where S: ServerStreamingService<T::Decode, Response = T::Encode>
```

Same as unary but service returns a response stream instead of single message.
Compression override is disabled for streams (per-message override not supported).

### Client Streaming

```rust
pub async fn client_streaming<S, B>(&mut self, service: S, req: http::Request<B>) -> http::Response<Body>
where S: ClientStreamingService<T::Decode, Response = T::Encode>
```

1. `map_request_streaming`: wrap body as `Streaming<T::Decode>`
2. Call service with `Request<Streaming<T::Decode>>`
3. Response is single message -> wrap in `tokio_stream::once(Ok(m))`

### Bidirectional

```rust
pub async fn streaming<S, B>(&mut self, service: S, req: http::Request<B>) -> http::Response<Body>
where S: StreamingService<T::Decode, Response = T::Encode>
```

1. `map_request_streaming`: wrap body as `Streaming<T::Decode>`
2. Call service -> response stream
3. `map_response` with the response stream

## Internal Methods

### `map_request_unary` (for unary + server-streaming)

```rust
async fn map_request_unary<B>(&mut self, request: http::Request<B>)
    -> Result<Request<T::Decode>, Status>
```

1. Check request compression encoding against accepted encodings
2. Split request into (parts, body)
3. Create `Streaming::new_request(decoder, body, encoding, max_size)`
4. `stream.try_next()` -> get single message (error if missing)
5. `stream.trailers()` -> merge trailing metadata into request
6. Return `Request::from_http_parts(parts, message)`

### `map_request_streaming` (for client-streaming + bidi)

```rust
fn map_request_streaming<B>(&mut self, request: http::Request<B>)
    -> Result<Request<Streaming<T::Decode>>, Status>
```

1. Check request compression encoding
2. `request.map(|body| Streaming::new_request(...))`
3. `Request::from_http(request)`

### `map_response` (all patterns)

```rust
fn map_response<B>(&mut self, response: Result<Response<B>, Status>, ...)
    -> http::Response<Body>
where B: Stream<Item = Result<T::Encode, Status>> + Send + 'static
```

1. Unwrap response (on error, return `status.into_http()` via `t!` macro)
2. Convert to HTTP: `response.into_http().into_parts()`
3. Insert `content-type: application/grpc`
4. Insert `grpc-encoding` header if compression accepted
5. Wrap body in `EncodeBody::new_server(encoder, body, ...)`
6. Return `http::Response::from_parts(parts, Body::new(body))`

## `t!` Macro

```rust
macro_rules! t {
    ($result:expr) => {
        match $result {
            Ok(value) => value,
            Err(status) => return status.into_http(),
        }
    };
}
```

Shortcut to return an error HTTP response immediately when an operation fails.
Uses `Status::into_http()` which creates a trailers-only response.

## Notes for Our Implementation

1. **Pattern: single struct handles all 4 patterns** — clean API
2. **Unary and server-streaming share `map_request_unary`** — both receive a single message
3. **Client-streaming and bidi share `map_request_streaming`** — both receive a stream
4. **All responses go through `map_response`** — wraps in EncodeBody, sets headers
5. **Server always sends trailers** — via `EncodeBody::new_server`
6. Skip compression initially — just pass `None` for encoding
7. **Error responses use trailers-only format** — `status.into_http()` sets
   grpc-status in initial headers with empty body
