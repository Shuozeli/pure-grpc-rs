# Code Quality Findings — pure-grpc-rs

Adversarial audit performed 2026-03-19 (second pass). Merged with prior audit (2026-03-18).
Phase 3 fixes applied 2026-03-19.

---

## Resolved (Prior Audit)

Issues found and fixed during the first audit cycle.

| # | Issue | Resolution |
|---|-------|------------|
| 1 | `unsafe advance_mut` in encode.rs — buffer not truncated on error | Fixed: `buf.truncate(offset)` on encode error (encode.rs:142-146) |
| 2 | `encode().ok()` / silent decode in reflection builder | Fixed: panics with descriptive context messages |
| 3 | Base64 decode failure silently produces empty Bytes | Fixed: returns `Status::Internal` on corrupt `grpc-status-details-bin` |
| 4 | TLS handshake errors logged at `debug!` only | Fixed: upgraded to `warn!` |
| 5 | `.expect()` in `Status::into_http` | Fixed: graceful fallback sends code-only on encoding failure |
| 6 | FQN formatting duplicated 3x in reflection | Fixed: extracted `make_fqn()` helper |
| 7 | Reflection doesn't index nested messages/enums | Fixed: recursive `index_message_types()` |
| 8 | `Status.clone()` + `std::mem::replace` in decode.rs | Fixed: direct assignment |
| 9 | HTTP 404 mapped to gRPC `Unimplemented` | Fixed: maps to `Code::NotFound` |
| 10 | Empty error message in router fallback | Fixed: `"service not found"` |
| 11 | Inconsistent error handling across server Grpc methods | Fixed: all use `status.into_http()` consistently |
| 12 | `MetadataMap` missing compression headers in reserved list | Fixed: includes `grpc-encoding`, `grpc-accept-encoding`, `grpc-status-details-bin` |
| 13 | Incomplete `Debug` impl for client `Grpc` | Fixed: includes all fields |
| 14 | Misleading "Return empty" comment in reflection | Fixed: comment matches behavior |
| 15 | `.expect()` panics in client `prepare_request` | Fixed: returns `Result<_, Status>` with error context |
| 16 | Hardcoded prost in health/reflection | Fixed: prost behind `prost-codec` feature flag (default-on) with `compile_error!` guard |

---

## Open — Critical (Blocks CI / Correctness)

### C1. Rustfmt violations across 13+ files
- `grpc-client/src/endpoint.rs:91, 102, 178`
- `grpc-client/src/grpc.rs:233, 243`
- `grpc-codegen/src/server_gen.rs:506`
- `grpc-core/src/codec/compression.rs:116`
- `grpc-core/src/metadata.rs:129`
- `grpc-core/src/status.rs:145, 325`
- `grpc-reflection/src/lib.rs:389`
- `grpc-server/src/grpc.rs:425`
- `examples/greeter-fbs/tests/integration.rs:78`
- **Fix:** `cargo fmt --all`

### C2. Unused import warning fails clippy
- `grpc-core/src/body.rs:92` — `use http_body::Body as _;`
- **Fix:** Remove or use the trait.

### C3. Status code overwritten on message decode failure
- `grpc-core/src/status.rs:230-235` (`Status::from_header_map`)
- If `grpc-message` header has invalid UTF-8, the original status code is replaced with `Code::Unknown`.
- **Fix:** Keep original code: `(code, msg)` instead of `(Code::Unknown, msg)`.

### C4. Server concurrency_limit(0) deadlocks all connections
- `grpc-server/src/server.rs:62-65`
- Semaphore with 0 permits blocks everything.
- **Fix:** Assert `limit > 0` or return Result.

---

## Open — High (Architecture / Significant Quality)

### H1. BoxFuture/BoxStream type aliases duplicated 7+ times
- `grpc-server/src/router.rs:12`
- `grpc-server/src/interceptor.rs:10`
- `grpc-health/src/lib.rs:144-145`
- `grpc-reflection/src/lib.rs:46-47`
- `examples/greeter-proto/src/server.rs:10-11`
- `examples/greeter-proto/tests/integration.rs:15-16`
- `examples/greeter-fbs/src/lib.rs:86`
- **Fix:** Define in `grpc-core` and re-export everywhere.

### H2. Excessive cloning of file_bytes in grpc-reflection
- `grpc-reflection/src/lib.rs:67, 75, 122, 132, 138, 151, 255, 265`
- Every symbol gets its own `Vec<u8>` clone of the entire file descriptor.
- **Fix:** Use `Arc<[u8]>` to share ownership.

### H3. GrpcService trait is a no-op wrapper of tower::Service
- `grpc-client/src/grpc.rs:16-49`
- Trait adds zero behavior; blanket impl delegates 1:1.
- **Fix:** Remove the trait, use `tower_service::Service` directly.

### H4. Manual Clone impl where derive suffices
- `grpc-client/src/grpc.rs:326-337` (`Grpc<T>`)
- **Fix:** Replace with `#[derive(Clone)]`.

### H5. Inconsistent error handling strategy in codegen
- `grpc-codegen/src/protobuf.rs:19` — `unwrap_or_default()` (silent empty)
- `grpc-codegen/src/protobuf.rs:40, 49, 57` — `expect()` (panic with message)
- `grpc-codegen/src/client_gen.rs:85, 89, 93` — `panic!()` via `unwrap_or_else`
- **Fix:** Standardize on returning `Result` throughout codegen.

### H6. Git dependencies unpinned
- `grpc-codegen/Cargo.toml` — `protoc-rs-schema`, `flatc-rs-schema`
- `grpc-build/Cargo.toml` — 4+ git deps, all unpinned
- **Fix:** Add `branch = "main"` per dependency management rules.

### H7. Clippy not run with --all-features in CI
- `.github/workflows/ci.yml`
- **Fix:** Add `cargo clippy --workspace --all-features -- -D warnings` step.

### H8. Metadata cloned on every status serialization
- `grpc-core/src/status.rs:281-282`
- `self.0.metadata.clone().into_sanitized_headers()` — O(n) per response.
- **Fix:** Provide reference-based iteration or consume self.

---

## Open — Medium

### Soundness

**M1. Body uses UnsyncBoxBody but is used across Send boundaries**
- `grpc-core/src/body.rs:5`
- Works because `Body::new` requires `B: Send + 'static`, but type system doesn't enforce it.
- **Fix:** Use `BoxBody` instead of `UnsyncBoxBody`.

### Silent Failures

**M2. Grpc::new creates client with empty/default URI**
- `grpc-client/src/grpc.rs:65-67` — `Uri::default()` produces unusable URI.
- **Fix:** Document requirement for `with_origin` or validate at construction.

**M3. compress/decompress are unreachable when gzip feature disabled**
- `grpc-core/src/codec/compression.rs:128-171`
- Not a real bug (empty enum makes match unreachable), but confusing.

**M4. FlatBuffers decode silently defaults missing fields**
- `examples/greeter-fbs/src/lib.rs:42` — `req.name().unwrap_or("")`
- **Fix:** Propagate error or document.

### Missing Validation

**M5. Max message sizes accept any usize including zero**
- `grpc-server/src/grpc.rs:64-72`
- **Fix:** Add bounds checking.

**M6. Endpoint timeout accepts Duration::ZERO**
- `grpc-client/src/endpoint.rs:55-63`
- **Fix:** Assert non-zero or document.

**M7. Service name not validated in codegen (empty generates invalid code)**
- `grpc-codegen/src/protobuf.rs:19` — `unwrap_or_default()`
- **Fix:** Validate non-empty.

**M8. gRPC path not validated in codegen IR**
- `grpc-codegen/src/ir.rs:52-54`
- **Fix:** Validate `proto_name` characters.

**M9. FlatBuffers index access without bounds checking**
- `grpc-codegen/src/flatbuffers.rs:40-42`
- **Fix:** Use `.get()` with error handling.

**M10. Reflection builder panics on invalid descriptor bytes**
- `grpc-reflection/src/lib.rs:108-117` — `expect()` in builder.
- **Fix:** Return `Result`.

### Performance

**M11. Status cloned on stream error (clone + return)**
- `grpc-core/src/codec/decode.rs:245-247`
- **Fix:** Move into state, clone only for return.

**M12. Double clone in protobuf service_from_proto**
- `grpc-codegen/src/protobuf.rs:19-27`
- **Fix:** Remove extra `.clone()`.

**M13. Triple clone in flatbuffers service_from_fbs**
- `grpc-codegen/src/flatbuffers.rs:32-34`
- **Fix:** Clone directly into struct fields.

**M14. URI cloned 3x in Endpoint::connect TLS match**
- `grpc-client/src/endpoint.rs:92-96`
- **Fix:** Clone once before the match.

**M15. Semaphore Arc cloned twice in accept loop**
- `grpc-server/src/server.rs:178-182`
- **Fix:** Clone once outside the match.

**M16. O(n^2) header merge loop in interceptor**
- `grpc-server/src/interceptor.rs:93-97`

### Logic / Correctness

**M17. Codegen stream type naming collision**
- `grpc-codegen/src/server_gen.rs:354-365`
- Method `say_hello_stream` yields `SayHelloStreamStream`. Test codifies the bug.

**M18. Double-unwrap `??` in Endpoint::connect**
- `grpc-client/src/endpoint.rs:109-110`
- Confusing; swallows original connection error.
- **Fix:** Use `.and_then(|r| r)`.

**M19. Endpoint `timeout`/`connect_timeout` not applied**
- `grpc-client/src/endpoint.rs:87`
- Fields accepted by builder but silently ignored during `connect()`.

**M20. HTTPS URI handling when `tls` feature disabled**
- `grpc-client/src/endpoint.rs:26-41`
- Should fail loudly.

### Incomplete Implementations

**M21. TLS example stubs 3 of 4 RPC patterns**
- `examples/greeter-proto/src/tls_server.rs:27-48`
- Server-streaming, client-streaming, bidi all return `Unimplemented`.

**M22. FlatBuffers example only supports unary**
- `examples/greeter-fbs/src/lib.rs:74-151`

**M23. Comments field in codegen IR never populated**
- `grpc-codegen/src/protobuf.rs:31, 69` — `comments: vec![]` always.

**M24. Missing Response::from_http_parts**
- `grpc-core/src/response.rs` — API asymmetry with Request.

---

## Open — Low (Polish)

### Code Duplication (Minor)
- **L1.** Request/Response struct impls ~90% identical (`request.rs` vs `response.rs`)
- **L2.** Identical test helper `module_items()` in codegen (`client_gen.rs:229` / `server_gen.rs:327`)
- **L3.** Duplicated HTTP/HTTPS request code in `Channel::call` (`channel.rs:150-167`)
- **L4.** `MockService` test struct duplicated 3 times in grpc-server
- **L5.** Four-way streaming match repeated 3 times in codegen

### Unsafe / Documentation
- **L6.** Unsafe `advance_mut` lacks `// SAFETY:` comment (`encode.rs:121-122`)
- **L7.** Unused `_buffer_settings` parameter in `decode_chunk` (`decode.rs:174-176`)
- **L8.** Unused `_service_name` parameter in `gen_dispatch_arm` (`server_gen.rs:231`)

### Decorative Noise
- **L9.** Section divider comments in health/reflection (8 locations)

### Test Quality
- **L10.** Test panics instead of assertions in reflection (`lib.rs:362, 432`)
- **L11.** Fragile string-based assertions in codegen (`server_gen.rs:509-521`)
- **L12.** Integration test `wait_for_server` only checks TCP, not gRPC readiness
- **L13.** Missing error path tests (endpoint, status, codegen)

### Logging / Observability
- **L14.** Connection errors logged without remote address (`server.rs:239`)
- **L15.** No tracing spans on spawned connection tasks (`server.rs:194-211`)

### Build Config
- **L16.** No `deny.toml` for security/license auditing
- **L17.** Inconsistent error message formatting across crates

---

## Summary

| | Resolved | Critical | High | Medium | Low |
|--|----------|----------|------|--------|-----|
| CI / Formatting | 0 | 2 | 1 | 0 | 0 |
| Correctness | 5 | 1 | 0 | 4 | 0 |
| Architecture | 1 | 0 | 4 | 0 | 0 |
| Silent Failures | 3 | 1 | 0 | 3 | 0 |
| Validation | 0 | 1 | 0 | 6 | 0 |
| Performance | 1 | 0 | 1 | 6 | 0 |
| Incomplete | 2 | 0 | 0 | 4 | 0 |
| Duplication | 1 | 0 | 1 | 0 | 5 |
| Build / Config | 0 | 0 | 2 | 0 | 2 |
| Code Quality | 3 | 0 | 0 | 0 | 8 |
| Tests | 0 | 0 | 0 | 0 | 4 |
| **Total** | **16** | **4** | **8** | **24** | **17** |
