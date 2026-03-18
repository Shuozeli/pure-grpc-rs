# Server Transport

**Source:** `tonic/src/transport/server/` (2,035 lines across 8 files)
**Dependencies:** `hyper` (1.x), `hyper-util`, `tower`, `tokio`

## Overview

The Server builder configures and runs an HTTP/2 server using hyper. It accepts
TCP connections, optionally wraps them in TLS, and dispatches requests through
a per-connection service stack.

## Server Type

```rust
#[derive(Clone)]
pub struct Server<L = Identity> {
    concurrency_limit: Option<usize>,
    load_shed: bool,
    timeout: Option<Duration>,
    tls: Option<TlsAcceptor>,
    init_stream_window_size: Option<u32>,
    init_connection_window_size: Option<u32>,
    max_concurrent_streams: Option<u32>,
    tcp_keepalive: Option<Duration>,
    tcp_nodelay: bool,                    // default: true
    http2_keepalive_interval: Option<Duration>,
    http2_keepalive_timeout: Duration,    // default: 20s
    max_frame_size: Option<u32>,
    accept_http1: bool,                   // default: false
    service_builder: ServiceBuilder<L>,
    max_connection_age: Option<Duration>,
    max_connection_age_grace: Option<Duration>,
    // ... more options
}
```

## Serve Flow

### 1. Bind & Listen

```rust
Server::builder()
    .add_service(greeter_service)
    .serve(addr)
    .await?;
```

`serve(addr)` creates a `TcpIncoming` listener, then calls `serve_with_incoming`.

### 2. Accept Loop

```rust
loop {
    tokio::select! {
        _ = &mut signal => break,           // shutdown signal
        io = incoming.next() => {
            let svc = make_svc.call(&io);   // per-connection service
            serve_connection(io, svc, ...);  // spawn connection task
        }
    }
}
```

### 3. Per-Connection Service Stack (MakeSvc)

Each TCP connection gets its own service instance:

```
RecoverErrorLayer    (recovers service failures)
→ LoadShedLayer      (optional, rejects when not ready)
→ ConcurrencyLimit   (optional, limits in-flight requests)
→ GrpcTimeout        (request timeout enforcement)
→ ConnectInfoLayer   (injects connection metadata into extensions)
→ Svc                (trace span injection + inner service)
```

### 4. HTTP/2 Connection (serve_connection)

```rust
let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor);
builder.http2()
    .initial_connection_window_size(...)
    .initial_stream_window_size(...)
    .max_concurrent_streams(...)
    .keep_alive_interval(...)
    .keep_alive_timeout(...)
    .max_frame_size(...)
    ...;

// Each connection runs as a tokio task:
tokio::spawn(async {
    tokio::select! {
        _ = builder.serve_connection(io, svc) => {},
        _ = connection_timeout => {},    // max_connection_age
        _ = shutdown_signal => {},       // graceful shutdown
    }
});
```

### 5. Graceful Shutdown

Uses `tokio::sync::watch` channel:
1. Accept loop breaks on shutdown signal
2. Broadcasts to all connection tasks via `watch::Sender`
3. Waits for all receivers to drop (all connections closed)
4. Optional `max_connection_age_grace` for forceful shutdown

## TcpIncoming (incoming.rs)

```rust
pub struct TcpIncoming {
    inner: TcpListenerStream,
    nodelay: Option<bool>,
    keepalive: Option<TcpKeepalive>,
    // ...
}

impl Stream for TcpIncoming {
    type Item = io::Result<TcpStream>;
    fn poll_next(...) {
        // Accept connection, then apply socket options (nodelay, keepalive)
    }
}
```

Socket options are applied **after** accept, not on the listener.

## Connected Trait (conn.rs)

```rust
pub trait Connected {
    type ConnectInfo: Clone + Send + Sync + 'static;
    fn connect_info(&self) -> Self::ConnectInfo;
}
```

- `TcpStream` -> `TcpConnectInfo { local_addr, remote_addr }`
- `TlsStream<T>` -> `TlsConnectInfo<T::ConnectInfo>` (includes peer certs)

Inserted into `http::Extensions` so handlers can access `request.remote_addr()`.

## Notes for Our Implementation

1. **Start simple**: bind TCP, accept connections, run hyper HTTP/2
2. **Skip TLS** initially — add in Phase 5
3. **Skip load shedding, connection age, and concurrent limits** initially
4. **Must have**: TCP accept loop, hyper HTTP/2 builder, per-connection task spawning
5. **Graceful shutdown** is nice-to-have for Phase 1 but not critical
6. **The MakeSvc pattern** (per-connection service factory) is important for
   connection info injection. Can simplify to just cloning the service.
7. **Minimal Phase 1 server**:
   ```rust
   let listener = TcpListener::bind(addr).await?;
   loop {
       let (stream, _) = listener.accept().await?;
       let svc = my_service.clone();
       tokio::spawn(async move {
           let builder = http2::Builder::new(TokioExecutor::new());
           builder.serve_connection(TokioIo::new(stream), svc).await;
       });
   }
   ```
