# Router

**Source:** `tonic/src/service/router.rs` (178 lines)
**Dependencies:** `axum`, `tower`

## Overview

Routes incoming gRPC requests to the appropriate service by path.
Tonic uses axum's Router internally. We plan to use a simpler
HashMap-based approach (no axum dependency).

## Type Definition

```rust
pub struct Routes {
    router: axum::Router,
}
```

## Routing Pattern

Services are registered by their `NamedService::NAME`:

```rust
self.router.route_service(
    &format!("/{}/{{*rest}}", S::NAME),  // e.g., "/helloworld.Greeter/{*rest}"
    svc.map_request(|req: Request<axum::body::Body>| req.map(Body::new)),
);
```

Pattern: `/{package.Service}/*rest` — the `*rest` captures the method name.

## Key Methods

```rust
Routes::new(svc) -> Self              // create with first service
Routes::default() -> Self             // empty with UNIMPLEMENTED fallback
    .add_service(svc) -> Self         // register a service
    .prepare() -> Self                // optimize router internals

RoutesBuilder::default() -> Self      // mutable builder
    .add_service(&mut self, svc) -> &mut Self
    .routes(self) -> Routes
```

## Service Requirements

Services must implement:
- `Service<Request<Body>, Error = Infallible>` — infallible (errors go through Status)
- `NamedService` — provides the routing path
- `Clone + Send + Sync + 'static` — for axum compatibility
- Response must implement `axum::response::IntoResponse`

## Fallback

```rust
async fn unimplemented() -> Response<Body> {
    let (parts, ()) = Status::unimplemented("").into_http::<()>().into_parts();
    Response::from_parts(parts, Body::empty())
}
```

Unknown paths return `grpc-status: 12 (UNIMPLEMENTED)`.

## Routes as tower::Service

```rust
impl<B> Service<Request<B>> for Routes
where B: Body<Data = Bytes> + Send + 'static, B::Error: Into<BoxError>
{
    type Response = Response<Body>;
    type Error = Infallible;  // never errors at the routing level
    ...
}
```

## Notes for Our Implementation

1. **Skip axum dependency** — use a `HashMap<String, Arc<dyn Service>>` instead
2. **Route pattern**: extract service name from URI path `/{service_name}/{method_name}`
3. **Fallback** to `Status::unimplemented("")` for unknown services
4. **Infallible error type** — the router itself never fails, errors are encoded
   as gRPC status in the response body/trailers
5. Our router should implement `tower::Service<http::Request<Body>>` for
   compatibility with hyper's serve loop
