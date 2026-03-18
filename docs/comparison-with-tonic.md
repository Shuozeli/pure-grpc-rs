# pure-grpc-rs vs tonic — Side-by-Side Comparison

## Overview

| | pure-grpc-rs | tonic |
|---|---|---|
| **Purpose** | Pure Rust gRPC framework with pluggable codecs | The de facto Rust gRPC framework |
| **Maturity** | New (2026) | Established (2019, v0.14) |
| **Serialization** | Pluggable: Protobuf (prost) + FlatBuffers | Protobuf-only (prost or protobuf crate) |
| **Dependencies** | hyper 1.x, tower, h2, tokio | hyper 1.x, tower, h2, tokio, axum (router) |
| **MSRV** | 2021 edition | 2024 edition, Rust 1.88 |
| **License** | MIT | MIT |

## Size Comparison

| Component | pure-grpc-rs | tonic | Ratio |
|-----------|-------------|-------|-------|
| **Total LOC** | 7,860 | 26,233 | **0.30x** |
| **Core LOC** | 7,860 | 15,035 | **0.52x** |
| **Crates** | 10 | 8+ | similar |
| **Rust files** | 47 | 98 | **0.48x** |
| **Tests** | 100 | N/A | — |

## Per-Module LOC

| Module | pure-grpc-rs | tonic | Notes |
|--------|-------------|-------|-------|
| Status/Code | 699 | 1,091 | Tonic has h2 error mapping, we have io::Error chain walk |
| Metadata | 160 | 4,286 | Tonic has full phantom-type Ascii/Binary system; we wrap HeaderMap directly |
| Codec/Framing | 1,141 | 1,461 | Similar — both have encode/decode state machines |
| Server handler | 1,202 | 645 | We include transport (server.rs); tonic separates it |
| Client dispatcher | 773 | 540 | We include transport (channel.rs); tonic separates it |
| Transport | (embedded) | 5,174 | Tonic has separate transport layer with reconnect, TLS, connection pooling |
| Codegen | 978 | 1,973 | Tonic has more options (attributes, well-known types, manual builder) |
| Health | 220 | 951 | Tonic has Watch RPC + generated proto; we have Check only |
| Reflection | 460 | 1,904 | Tonic supports v1 + v1alpha; we support v1 only |

## Feature Comparison

| Feature | pure-grpc-rs | tonic |
|---------|-------------|-------|
| Unary RPC | Yes | Yes |
| Server streaming | Yes | Yes |
| Client streaming | Yes | Yes |
| Bidirectional streaming | Yes | Yes |
| Protobuf (prost) codec | Yes (feature-gated) | Yes (default) |
| FlatBuffers codec | Yes | No |
| Pluggable codecs | Yes (Codec trait) | Yes (Codec trait) |
| TLS (rustls) | Yes (feature-gated) | Yes (feature-gated) |
| gzip compression | Yes (feature-gated) | Yes (feature-gated) |
| zstd compression | No | Yes |
| deflate compression | No | Yes |
| Graceful shutdown | Yes | Yes |
| Request timeouts | Yes | Yes |
| Health checking | Yes (Check RPC) | Yes (Check + Watch RPCs) |
| Server reflection | Yes (v1) | Yes (v1 + v1alpha) |
| Interceptors | Yes | Yes |
| Tower middleware | Yes (via tower::Service) | Yes (via tower::Service) |
| Connection pooling | No (hyper handles it) | Yes (tower::Buffer) |
| Reconnection | Automatic (hyper-util) | Explicit state machine |
| Load balancing | No | No (separate `grpc` crate) |
| gRPC-Web | No | Yes (tonic-web) |
| Router | HashMap-based | Axum-based |
| Code generation | Yes (protobuf + flatbuffers) | Yes (protobuf only) |
| build.rs integration | `compile_protos()` + `compile_fbs()` | `compile_protos()` |
| File descriptor set | Auto-generated for reflection | Manual configuration |

## Architecture Differences

### Router
- **tonic**: Uses axum `Router` with `/{SERVICE_NAME}/*rest` patterns
- **pure-grpc-rs**: Uses `HashMap<String, Arc<dyn Service>>` with manual path parsing. Simpler, no axum dependency.

### Metadata
- **tonic**: Full phantom-type system (`MetadataKey<Ascii>`, `MetadataKey<Binary>`) with `repr(transparent)` for zero-cost transmutes. 4,286 lines.
- **pure-grpc-rs**: Thin wrapper over `http::HeaderMap`. 160 lines. Trade-off: no compile-time Ascii/Binary enforcement.

### Transport
- **tonic**: Separate `transport` module (5,174 lines) with `Channel` backed by `tower::Buffer`, `Endpoint` builder, `Reconnect` state machine, `AddOrigin`/`UserAgent` layers.
- **pure-grpc-rs**: Transport embedded in server.rs (hyper serve loop) and channel.rs (hyper-util Client). Simpler but less configurable.

### Codec System
Both use the same pattern:
```rust
pub trait Codec {
    type Encode; type Decode;
    type Encoder: Encoder; type Decoder: Decoder;
    fn encoder(&mut self) -> Self::Encoder;
    fn decoder(&mut self) -> Self::Decoder;
}
```
Key difference: pure-grpc-rs has `FlatBuffersCodec` out of the box.

### Code Generation
- **tonic**: Generic `Service`/`Method` traits in `tonic-build`, with `tonic-prost-build` as the protobuf adapter
- **pure-grpc-rs**: `ServiceDef`/`MethodDef` IR in `grpc-codegen`, with both protobuf and flatbuffers adapters. Also integrated directly into `protoc-rs-codegen` and `flatc-rs-codegen` via feature flags.

## What tonic has that we don't
1. **gRPC-Web protocol** (tonic-web)
2. **zstd/deflate compression**
3. **Health Watch RPC** (streaming health updates)
4. **Reflection v1alpha** (legacy support)
5. **Richer error model** (tonic-types, google.rpc.Status details)
6. **Advanced load balancing** (tonic-xds, round-robin, pick-first)
7. **Extensive metadata API** (Ascii/Binary phantom types, 4K lines)

## What we have that tonic doesn't
1. **FlatBuffers codec** (pluggable, first-class)
2. **FlatBuffers build.rs** (`compile_fbs()`)
3. **Integrated codegen in compilers** (protobuf-rs and flatbuffers-rs both have `grpc` feature)
4. **Auto-generated FILE_DESCRIPTOR_SET** constant for reflection
5. **Simpler codebase** (0.30x the size)
