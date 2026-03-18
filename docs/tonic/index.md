# Tonic Architecture Study

Study of [tonic](https://github.com/hyperium/tonic) (v0.14.5) as reference for
building pure-grpc-rs. Vendor source at `vendor/tonic/`.

## Crate Map

| Crate | LOC | Files | Purpose | Priority |
|-------|-----|-------|---------|----------|
| tonic (core) | 15,035 | 47 | Core gRPC over HTTP/2 | **P0** |
| tonic-build | 1,973 | 5 | Codegen framework (traits + templates) | P1 |
| tonic-prost | 437 | 2 | Prost codec implementation | P1 |
| tonic-prost-build | 1,281 | 2 | Prost build.rs integration | P2 |
| tonic-protobuf | 125 | 1 | Protobuf codec | P2 |
| grpc (new) | 17,509 | 51 | Advanced client (load balancing, xDS) | P3 (skip) |
| tonic-health | 951 | 4 | Health checking service | P3 |
| tonic-reflection | 1,904 | 8 | Server reflection | P3 |
| tonic-web | 1,426 | 5 | gRPC-Web protocol translation | P3 (skip) |
| tonic-types | 4,652 | 12 | Richer error model | P3 |

## Component Study Order

We study tonic's core crate bottom-up: foundational types first, then
framing/codec, then server/client, then transport. Each component gets its
own doc file with type signatures, design decisions, and notes for our impl.

### Layer 1: Foundational Types

1. [Status & Code](./01-status.md) — Error model, gRPC↔HTTP mapping
2. [Metadata](./02-metadata.md) — MetadataMap, Ascii/Binary encoding
3. [Request & Response](./03-request-response.md) — Wrapper types, extensions
4. [Body](./04-body.md) — Type-erased HTTP body

### Layer 2: Codec & Framing

5. [Codec Traits](./05-codec-traits.md) — Encoder/Decoder/Codec, BufferSettings
6. [Encode (Framing)](./06-encode.md) — Frame encoding, compression, streaming
7. [Decode (Streaming)](./07-decode.md) — State machine decoder, Streaming<T>
8. [ProstCodec](./08-prost-codec.md) — Prost codec implementation

### Layer 3: Server & Client

9. [Server Handler](./09-server-handler.md) — Grpc<T> server-side dispatch
10. [Service Traits](./10-service-traits.md) — UnaryService, StreamingService, etc.
11. [Client Dispatcher](./11-client-dispatcher.md) — Grpc<T> client-side calls
12. [Router](./12-router.md) — Service routing

### Layer 4: Transport

13. [Channel & Endpoint](./13-channel-endpoint.md) — Client transport
14. [Server Transport](./14-server-transport.md) — Hyper HTTP/2 serve loop

### Layer 5: Codegen

15. [tonic-build](./15-tonic-build.md) — Service/Method traits, code templates
16. [tonic-prost-build](./16-tonic-prost-build.md) — Prost integration

## Dependency Graph (Core Crate)

```
                    ┌─────────────────────────────────────────┐
                    │              transport                   │
                    │  ┌─────────────┐  ┌──────────────────┐  │
                    │  │   Channel   │  │      Server      │  │
                    │  │  (Endpoint) │  │  (hyper serve)   │  │
                    │  └──────┬──────┘  └────────┬─────────┘  │
                    └─────────┼──────────────────┼────────────┘
                              │                  │
                    ┌─────────┼──────────────────┼────────────┐
                    │         ▼                  ▼             │
                    │  ┌─────────────┐  ┌──────────────────┐  │
                    │  │   client/   │  │     server/      │  │
                    │  │   grpc.rs   │  │     grpc.rs      │  │
                    │  └──────┬──────┘  └────────┬─────────┘  │
                    └─────────┼──────────────────┼────────────┘
                              │                  │
                    ┌─────────┼──────────────────┼────────────┐
                    │         ▼                  ▼             │
                    │  ┌──────────────────────────────────┐   │
                    │  │         codec/ (framing)          │   │
                    │  │  encode.rs  decode.rs  buffer.rs  │   │
                    │  └──────────────┬───────────────────┘   │
                    │                 │                        │
                    │         ┌───────┴────────┐              │
                    │         │  Codec traits  │              │
                    │         │  mod.rs        │              │
                    │         └───────┬────────┘              │
                    └─────────────────┼───────────────────────┘
                                      │
                    ┌─────────────────┼───────────────────────┐
                    │                 ▼                        │
                    │  ┌──────────────────────────────────┐   │
                    │  │      Foundational Types           │   │
                    │  │  Status  Metadata  Req/Res  Body │   │
                    │  └──────────────────────────────────┘   │
                    └─────────────────────────────────────────┘
```

## External Dependencies (Core)

| Dep | Version | Used For |
|-----|---------|----------|
| bytes | 1.x | Zero-copy byte buffers |
| http | 1.x | HTTP types (Request, Response, HeaderMap) |
| http-body | 1.x | Body trait |
| http-body-util | 0.1 | Body combinators, UnsyncBoxBody |
| hyper | 1.x | HTTP/2 server/client |
| hyper-util | 0.1 | Server auto connection |
| h2 | 0.4 | HTTP/2 error types |
| tower / tower-service | 0.4/0.3 | Service trait, middleware |
| tokio | 1.x | Async runtime |
| tokio-stream | 0.1 | Stream utilities |
| futures-core | 0.3 | Stream trait |
| pin-project-lite | 0.2 | Pin projection |
| percent-encoding | 2.x | grpc-message header encoding |
| base64 | 0.22 | Binary metadata values |
| tracing | 0.1 | Logging |
| prost | 0.13 | Protobuf codec (optional) |
| axum | 0.7 | Router (optional, we skip) |
