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

## Resolved — Medium (2026-03-19, second pass)

| # | Issue | Resolution |
|---|-------|------------|
| M1 | Body uses UnsyncBoxBody | NOT A BUG — uses custom `SendBoxBody` with `Send + 'static` bound |
| M2 | Grpc::new creates client with empty URI | Fixed prior: tests require `with_origin` |
| M3 | compress/decompress unreachable without gzip | Fixed: clean signatures, documented uninhabited enum design |
| M4 | FlatBuffers decode silently defaults | Fixed: `.ok_or("field is required")?` error propagation |
| M5 | Max message sizes accept zero | Fixed prior: `assert!(limit > 0)` |
| M6 | Endpoint timeout accepts Duration::ZERO | Fixed prior: `assert!(!timeout.is_zero())` |
| M7 | Codegen panics instead of returning Result | Fixed: `service_from_proto` returns `Result<ServiceDef, String>` |
| M8 | gRPC path not validated | Fixed: `ServiceDef::validate()` checks names, slashes, empty fields |
| M9 | FlatBuffers index without bounds check | Fixed prior: `.get().expect()` |
| M10 | Reflection builder panics | Fixed: returns `Result<Self, Status>` |
| M11 | Status cloned on stream error | Accepted: clone is necessary, marked with `// TODO(refactor):` |
| M12 | Double clone in protobuf | Fixed prior |
| M13 | Triple clone in flatbuffers | Fixed prior |
| M14 | URI cloned 3x | Fixed prior |
| M15 | Semaphore Arc cloned twice | Fixed prior |
| M16 | O(n^2) header merge | NOT A BUG — code uses `std::mem::take`, no iteration |
| M17 | Stream type naming collision | Fixed: `response_stream_ident()` strips trailing "Stream" |
| M18 | Double-unwrap `??` | Fixed prior: `.and_then(\|r\| r)` |
| M19 | Timeouts not applied | NOT A BUG — applied via `channel.with_timeout()` |
| M20 | HTTPS without tls feature | Fixed prior: `panic!()` on https without tls |
| M23 | Comments never populated (protobuf) | Documented: requires `SourceCodeInfo` integration |
| M24 | Missing Response::from_http_parts | Fixed: added symmetric constructor + test |

### Open — Incomplete Implementations (deliberate scope limits)

**M21. TLS example stubs 3 of 4 RPC patterns**
- `examples/greeter-proto/src/tls_server.rs` — intentional: minimal TLS demo.

**M22. FlatBuffers example only supports unary**
- `examples/greeter-fbs/src/lib.rs` — intentional: minimal FlatBuffers demo.

---

## Resolved — Low (2026-03-19, second pass)

| # | Issue | Resolution |
|---|-------|------------|
| L2 | Duplicated `module_items()` test helper | Fixed: extracted to `grpc-codegen/src/test_util.rs` |
| L3 | Duplicated HTTP/HTTPS in Channel::call | Fixed: `send_request!` macro deduplicates branches |
| L6 | Unsafe `advance_mut` lacks SAFETY comment | Fixed: added `// SAFETY:` comment |
| L7 | Unused `_buffer_settings` parameter | STALE — parameter was already removed |
| L8 | Unused `_service_name` parameter | STALE — parameter is used |
| L14 | Connection errors without remote address | STALE — remote_addr is already logged |

### Open — Low (not worth changing)

- **L1.** Request/Response impls ~90% identical — dedup via macro would reduce readability
- **L4.** MockService duplicated 3x in grpc-server tests — each variant serves different purpose
- **L5.** Four-way streaming match repeated 3x in codegen — inherent to per-pattern token generation
- **L9.** Section divider comments — style preference
- **L10.** Test panics vs assertions in reflection — idiomatic Rust match-arm patterns
- **L11.** String-based codegen assertions — some already use syn AST parsing, rest are adequate
- **L12.** wait_for_server checks TCP only — acceptable for test reliability
- **L13.** Missing error path tests — incremental, not blocking
- **L15.** No tracing spans on connection tasks — polish
- **L16.** No deny.toml — orthogonal to code quality
- **L17.** Inconsistent error formatting — minor

---

## Summary

| | Resolved | Open |
|--|----------|------|
| Critical | 4 | 0 |
| High | 8 | 0 |
| Medium | 22 | 2 (deliberate scope) |
| Low | 6 | 11 (not worth changing) |
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
