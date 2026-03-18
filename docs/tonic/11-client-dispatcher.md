# Client Dispatcher

**Source:** `tonic/src/client/grpc.rs` (467 lines), `tonic/src/client/service.rs` (50 lines)
**Dependencies:** `http`, `http-body`, `tokio-stream`, `tower-service`

## Overview

`Grpc<T>` is the client-side dispatcher. `T` is the transport (typically
`Channel`). It encodes requests, sends them over HTTP/2, and decodes responses.

## Type Definitions

```rust
pub struct Grpc<T> {
    inner: T,               // the transport service (implements GrpcService)
    config: GrpcConfig,
}

struct GrpcConfig {
    origin: Uri,
    accept_compression_encodings: EnabledCompressionEncodings,
    send_compression_encodings: Option<CompressionEncoding>,
    max_decoding_message_size: Option<usize>,
    max_encoding_message_size: Option<usize>,
}
```

## GrpcService Trait

```rust
pub trait GrpcService<ReqBody> {
    type ResponseBody: Body;
    type Error: Into<BoxError>;
    type Future: Future<Output = Result<http::Response<Self::ResponseBody>, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;
    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future;
}
```

Blanket impl for any `tower::Service<http::Request<ReqBody>>` returning
`http::Response<ResBody>`. This is how `Channel` (tower service) works as transport.

## Builder Methods

```rust
Grpc::new(inner) -> Self                    // origin = Uri::default()
Grpc::with_origin(inner, origin) -> Self    // explicit origin URI
    .send_compressed(encoding) -> Self
    .accept_compressed(encoding) -> Self
    .max_decoding_message_size(limit) -> Self
    .max_encoding_message_size(limit) -> Self
```

## Four Call Methods

### Unary

```rust
pub async fn unary<M1, M2, C>(&mut self, request: Request<M1>, path: PathAndQuery, codec: C)
    -> Result<Response<M2>, Status>
```

Wraps message in `tokio_stream::once(m)`, delegates to `client_streaming`.

### Client Streaming

```rust
pub async fn client_streaming<S, M1, M2, C>(&mut self, request: Request<S>, path, codec)
    -> Result<Response<M2>, Status>
```

1. Calls `streaming()` to get `Response<Streaming<M2>>`
2. `body.try_next()` -> extract single response message
3. `body.trailers()` -> merge trailing metadata
4. Return `Response::from_parts(metadata, message, extensions)`

### Server Streaming

```rust
pub async fn server_streaming<M1, M2, C>(&mut self, request: Request<M1>, path, codec)
    -> Result<Response<Streaming<M2>>, Status>
```

Wraps message in `tokio_stream::once(m)`, delegates to `streaming`.

### Streaming (Core Implementation)

```rust
pub async fn streaming<S, M1, M2, C>(&mut self, request: Request<S>, path, codec)
    -> Result<Response<Streaming<M2>>, Status>
```

1. **Encode request body:**
   ```rust
   request.map(|s| EncodeBody::new_client(codec.encoder(), s.map(Ok), ...))
          .map(Body::new)
   ```
2. **Prepare HTTP request:** `config.prepare_request(request, path)`
3. **Send:** `self.inner.call(request).await`
4. **Create response:** `self.create_response(decoder, response)`

## Request Preparation (`GrpcConfig::prepare_request`)

1. Build URI: combine `origin` scheme/authority with `path`
   - If origin has non-root path, concatenate: `{origin_path}{method_path}`
   - Otherwise, use `path` directly
2. Convert to HTTP request: `request.into_http(uri, POST, HTTP_2, SanitizeHeaders::Yes)`
3. Add headers:
   - `te: trailers` (required by gRPC spec)
   - `content-type: application/grpc`
   - `grpc-encoding: {encoding}` (if compression)
   - `grpc-accept-encoding: {encodings}` (what client accepts)

## Response Handling (`create_response`)

1. Parse `grpc-encoding` from response headers -> check against accepted encodings
2. Check for **trailers-only response**: `Status::from_header_map(response.headers())`
   - If grpc-status present and non-OK: return `Err(status)` immediately
   - If grpc-status present and OK: create `Streaming::new_empty` (no body expected)
   - If no grpc-status: create `Streaming::new_response` (expect body + trailers)
3. Wrap in `Response::from_http(response)`

## Call Graph

```
unary         -> client_streaming -> streaming
server_streaming                  -> streaming
```

All four patterns funnel through `streaming()`, which is the core implementation.
`client_streaming` and `unary` extract a single message from the response stream.

## Notes for Our Implementation

1. **`streaming()` is the core** — other methods delegate to it
2. **Trailers-only detection** is critical — check grpc-status in initial headers
3. **`ready()` method** wraps `poll_ready` for tower backpressure
4. **`te: trailers`** header is required by HTTP/2 for gRPC
5. **URI construction** handles path concatenation for proxies/gateways
6. **Clone is cheap** — `Grpc<T: Clone>` clones the inner service (typically
   `Channel` backed by `tower::Buffer`)
7. **Codec is passed per-call** — not stored in Grpc struct (unlike server where
   it's stored). This is because generated client code creates codec per-method.
