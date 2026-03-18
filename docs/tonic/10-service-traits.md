# Service Traits

**Source:** `tonic/src/server/service.rs` (122 lines), `tonic/src/server/mod.rs` (26 lines)
**Dependencies:** `tower-service`

## Overview

Four traits for the four gRPC patterns. Each has a blanket impl for
`tower::Service` with matching signature, enabling tower middleware.

## Trait Definitions

### UnaryService

```rust
pub trait UnaryService<R> {
    type Response;
    type Future: Future<Output = Result<Response<Self::Response>, Status>>;
    fn call(&mut self, request: Request<R>) -> Self::Future;
}
```

Request: single message `R`
Response: single message wrapped in `Response<Self::Response>`

### ServerStreamingService

```rust
pub trait ServerStreamingService<R> {
    type Response;
    type ResponseStream: Stream<Item = Result<Self::Response, Status>>;
    type Future: Future<Output = Result<Response<Self::ResponseStream>, Status>>;
    fn call(&mut self, request: Request<R>) -> Self::Future;
}
```

Request: single message `R`
Response: stream of messages

### ClientStreamingService

```rust
pub trait ClientStreamingService<R> {
    type Response;
    type Future: Future<Output = Result<Response<Self::Response>, Status>>;
    fn call(&mut self, request: Request<Streaming<R>>) -> Self::Future;
}
```

Request: `Streaming<R>` (stream of messages)
Response: single message

### StreamingService (Bidirectional)

```rust
pub trait StreamingService<R> {
    type Response;
    type ResponseStream: Stream<Item = Result<Self::Response, Status>>;
    type Future: Future<Output = Result<Response<Self::ResponseStream>, Status>>;
    fn call(&mut self, request: Request<Streaming<R>>) -> Self::Future;
}
```

Request: `Streaming<R>` (stream of messages)
Response: stream of messages

## Blanket Implementations

Each trait has a blanket impl for tower `Service`:

```rust
impl<T, M1, M2> UnaryService<M1> for T
where T: Service<Request<M1>, Response = Response<M2>, Error = Status>
{ ... }

impl<T, S, M1, M2> ServerStreamingService<M1> for T
where T: Service<Request<M1>, Response = Response<S>, Error = Status>,
      S: Stream<Item = Result<M2, Status>>
{ ... }

// similar for ClientStreaming and Streaming
```

This means any tower middleware that wraps a `Service` automatically works
with gRPC services.

## NamedService

```rust
pub trait NamedService {
    const NAME: &'static str;
}
```

Used for routing — the `NAME` is the fully-qualified gRPC service name
(e.g., `"helloworld.Greeter"`). The router uses `"/{NAME}/*rest"` as the
path pattern.

## Notes for Our Implementation

1. **Keep all 4 traits** — they make the handler dispatch type-safe
2. **Blanket impls for tower::Service** are important for middleware composability
3. **NamedService** is used by the router — generated code sets this constant
4. These traits are what codegen implements. For hand-written services, implement
   these traits directly on your service struct.
