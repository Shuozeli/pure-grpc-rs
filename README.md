# pure-grpc-rs

A pure Rust gRPC framework with pluggable codecs. No tonic dependency.

Built on **hyper/h2/tower** for HTTP/2, with first-class support for both **Protobuf** (via prost) and **FlatBuffers** serialization.

[![CI](https://github.com/Shuozeli/pure-grpc-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Shuozeli/pure-grpc-rs/actions/workflows/ci.yml)

## Features

- All 4 gRPC patterns: unary, server-streaming, client-streaming, bidirectional
- Pluggable codecs: Protobuf (prost) and FlatBuffers out of the box
- Code generation from `.proto` files via [protobuf-rs](https://github.com/shuozeli/protobuf-rs)
- TLS with rustls (feature-gated)
- Compression: gzip, deflate, zstd (all feature-gated)
- gRPC-Web protocol support (binary and base64 modes)
- Rich error model (`google.rpc.*` detail types via grpc-types)
- Round-robin load balancing (BalancedChannel)
- Concurrency limiting (server-side, semaphore-based)
- Graceful shutdown
- Request timeouts
- Health checking service (`grpc.health.v1.Health`)
- Server reflection (v1)
- Interceptors for auth, logging, etc.
- grpcurl interop verified

## Quick Start

### 1. Define your proto

```protobuf
// proto/helloworld.proto
syntax = "proto3";
package helloworld;

service Greeter {
  rpc SayHello (HelloRequest) returns (HelloReply) {}
}

message HelloRequest { string name = 1; }
message HelloReply { string message = 1; }
```

### 2. Add build.rs

```rust
// build.rs
fn main() {
    grpc_build::compile_protos(
        &["proto/helloworld.proto"],
        &["proto"],
    ).unwrap();
}
```

### 3. Implement the server

```rust
use helloworld::greeter_server::{Greeter, GreeterServer};

struct MyGreeter;

impl Greeter for MyGreeter {
    fn say_hello(&self, request: Request<HelloRequest>)
        -> BoxFuture<Result<Response<HelloReply>, Status>>
    {
        let name = request.into_inner().name;
        Box::pin(async move {
            Ok(Response::new(HelloReply {
                message: format!("Hello, {name}!"),
            }))
        })
    }
    // ... streaming methods
}

#[tokio::main]
async fn main() {
    // Enable reflection for grpcurl service discovery
    let reflection = grpc_reflection::ReflectionServer::builder()
        .register_encoded_file_descriptor_set(helloworld::FILE_DESCRIPTOR_SET)
        .build();

    let router = Router::new()
        .add_service(GreeterServer::<MyGreeter>::NAME, GreeterServer::new(MyGreeter))
        .add_service(grpc_reflection::ReflectionServer::NAME, reflection);

    Server::builder()
        .serve_with_shutdown(addr, router, async { tokio::signal::ctrl_c().await.ok(); })
        .await
        .unwrap();
}
```

### 4. Use the client

```rust
let mut client = GreeterClient::connect("http://127.0.0.1:50051".parse()?).await?;
let response = client.say_hello(HelloRequest { name: "world".into() }).await?;
println!("{}", response.get_ref().message);
```

## Crates

| Crate | Description |
|-------|-------------|
| `grpc-core` | Status, Metadata, Codec traits, gRPC framing, Streaming, ProstCodec |
| `grpc-server` | Grpc handler, service traits, Router, HTTP/2 Server, interceptors |
| `grpc-client` | Grpc dispatcher, GrpcService trait, Channel, Endpoint, BalancedChannel |
| `grpc-codegen` | Service IR, server/client code generation |
| `grpc-build` | `compile_protos()` and `compile_fbs()` build.rs entry points |
| `grpc-codec-flatbuffers` | FlatBuffers codec + `FlatBufferGrpcMessage` trait |
| `grpc-health` | gRPC Health Checking service (Check + Watch RPCs) |
| `grpc-reflection` | gRPC Server Reflection service (v1) |
| `grpc-types` | Rich error model (`google.rpc.*` detail types, `StatusExt` trait) |
| `grpc-web` | gRPC-Web protocol translation layer (binary + base64 modes) |

## Optional Features

| Feature | Crate | Description |
|---------|-------|-------------|
| `prost-codec` | grpc-core | Protobuf codec via prost (default) |
| `gzip` | grpc-core, grpc-server, grpc-client | Gzip compression support |
| `deflate` | grpc-core, grpc-server, grpc-client | Deflate compression support |
| `zstd` | grpc-core, grpc-server, grpc-client | Zstd compression support |
| `tls` | grpc-server, grpc-client | TLS via rustls |

## Architecture

```
Application
  |
  +-- Generated Code (grpc-codegen + grpc-build)
  |     +-- XxxServer<T> (implements tower::Service)
  |     +-- XxxClient<T> (wraps grpc-client::Grpc<T>)
  |
  +-- grpc-web (tower layer, wraps server for browser clients)
  |
  +-- grpc-server
  |     +-- Grpc<T:Codec> handler (4 RPC patterns)
  |     +-- Router (HashMap-based path dispatch)
  |     +-- Server (hyper HTTP/2 + optional TLS)
  |     +-- InterceptedService
  |
  +-- grpc-client
  |     +-- Grpc<T> dispatcher (4 RPC patterns)
  |     +-- Channel (hyper HTTP/2 client, Clone)
  |     +-- BalancedChannel (round-robin load balancing)
  |     +-- Endpoint builder
  |
  +-- grpc-types (google.rpc.* rich error details, StatusExt)
  |
  +-- grpc-health (Health Check + Watch RPCs)
  +-- grpc-reflection (Server Reflection v1)
  |
  +-- grpc-core
        +-- Status/Code (gRPC error model)
        +-- MetadataMap (gRPC metadata)
        +-- Request/Response wrappers
        +-- Codec/Encoder/Decoder traits
        +-- Framing (5-byte header encode/decode)
        +-- Streaming<T> (state machine decoder)
        +-- Compression (gzip, deflate, zstd)
```

## Examples

| Example | Description | Run |
|---------|-------------|-----|
| `greeter-proto` | Protobuf greeter (all 4 RPC patterns, auto-generated from .proto) | `cargo run --bin greeter-server` / `cargo run --bin greeter-client` |
| `greeter-proto` (TLS) | TLS-encrypted protobuf greeter | `cargo run --bin greeter-tls-server --features tls` |
| `greeter-fbs` | FlatBuffers greeter (pluggable codec demo) | `cargo run --bin greeter-fbs-server` / `cargo run --bin greeter-fbs-client` |

## License

MIT
