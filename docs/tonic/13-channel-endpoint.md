# Channel & Endpoint

**Source:** `tonic/src/transport/channel/` (1,457 lines across 5 files + service/)
**Dependencies:** `hyper`, `hyper-util`, `tower` (Buffer, Service), `h2`, `tokio`

## Overview

`Channel` is the client-side HTTP/2 connection. It wraps the entire connection
in a `tower::Buffer` for cheap cloning and request multiplexing.
`Endpoint` is the builder for configuring and creating Channels.

## Channel

```rust
#[derive(Clone)]
pub struct Channel {
    svc: Buffer<Request<Body>, BoxFuture<'static, Result<Response<Body>, BoxError>>>,
}
```

**Key insight: `tower::Buffer`** wraps the connection in a background task
with an MPSC channel. Cloning the Channel creates a new handle to the same
background connection. This is how multiple concurrent RPCs share one HTTP/2 connection.

### Connection Modes

- `Channel::connect(endpoint)` — eager: connects immediately, fails if can't connect
- `Channel::connect_lazy(endpoint)` — lazy: defers connection to first request

### Service Implementation

```rust
impl Service<http::Request<Body>> for Channel {
    type Response = http::Response<Body>;
    type Error = BoxError;
    fn poll_ready(&mut self, cx) -> Poll<Result<(), Self::Error>> {
        self.svc.poll_ready(cx).map_err(Into::into)
    }
    fn call(&mut self, req) -> Self::Future {
        self.svc.call(req)
    }
}
```

Default buffer size: 1024.

## Endpoint

```rust
pub struct Endpoint {
    uri: EndpointType,         // Uri or Uds path
    origin: Option<Uri>,
    user_agent: Option<HeaderValue>,
    timeout: Option<Duration>,
    concurrency_limit: Option<usize>,
    rate_limit: Option<(u64, Duration)>,
    tls: Option<TlsConnector>,
    buffer_size: Option<usize>,
    init_stream_window_size: Option<u32>,
    init_connection_window_size: Option<u32>,
    tcp_keepalive: Option<Duration>,
    tcp_nodelay: bool,         // default: true
    http2_keep_alive_interval: Option<Duration>,
    http2_keep_alive_timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
    http2_adaptive_window: Option<bool>,
    executor: SharedExec,
    // ... more options
}
```

Builder pattern: all methods return `Self`.

### connect() Flow

1. Create HTTP connector with TCP settings
2. Wrap in `Connector` (adds optional TLS)
3. Build `Connection` with the service stack:
   ```
   AddOrigin          (inject scheme/authority)
   → UserAgent        (add user-agent header)
   → GrpcTimeout      (handle timeout)
   → ConcurrencyLimit (optional)
   → RateLimit        (optional)
   → Reconnect        (manages reconnection state machine)
   ```
4. Wrap in `tower::Buffer`
5. Return `Channel`

## Connection (service/connection.rs)

Uses hyper's HTTP/2 client:

```rust
// MakeSendRequestService creates connections:
// 1. Calls connector with URI
// 2. Performs HTTP/2 handshake: builder.handshake(io)
// 3. Spawns connection task as background worker
// 4. Returns SendRequest service for multiplexing
```

## Reconnect (service/reconnect.rs)

State machine:

```
    ┌─────┐   poll_ready()    ┌────────────┐   future ready    ┌───────────┐
    │ Idle │ ───────────────→ │ Connecting │ ─────────────────→ │ Connected │
    └─────┘                   └────────────┘                    └───────────┘
       ↑                          │ error                           │ error
       └──────────────────────────┘                                 │
       └────────────────────────────────────────────────────────────┘
```

Key distinction:
- **Lazy connections**: tolerate initial connection failures (store error)
- **Eager connections**: fail immediately on first connection error
- **Reconnection after established**: always tolerates (soft error)

## Notes for Our Implementation

1. **`tower::Buffer` is essential** — it solves the `&mut self` / cloning problem
2. **Start without reconnect** — connect once, fail on disconnect
3. **Start without TLS** — just plain HTTP/2
4. **Skip rate limiting and concurrency limiting** initially
5. **Keep the service stack concept** — AddOrigin is important for URI construction
6. **Endpoint builder can be minimal** — just uri, timeout, tcp_nodelay
7. For Phase 1, a simple Channel that holds a hyper HTTP/2 client wrapped
   in `tower::Buffer` is sufficient
