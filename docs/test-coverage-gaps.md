# Test Coverage Gaps — pure-grpc-rs

Audit date: 2026-03-22 (test count updated 2026-03-26). Current: 221 tests passing. Target: 95% branch coverage.

## Summary

| Crate | Current Est. | Missing Tests | Priority |
|-------|-------------|---------------|----------|
| grpc-core | ~70% | ~70-85 | Critical |
| grpc-server | ~55% | ~25-30 | Critical |
| grpc-client | ~30% | ~35-40 | Critical |
| grpc-codegen | ~80% | ~8-10 | Medium |
| grpc-build | ~50% | ~10-12 | High |
| grpc-codec-flatbuffers | ~70% | ~5-6 | Medium |
| grpc-health | ~85% | ~4-5 | Low |
| grpc-reflection | ~75% | ~8-10 | Medium |
| grpc-types | ~80% | ~6-8 | Medium |
| grpc-web | ~50% | ~10-12 | High |
| **Total** | **~65%** | **~200** | |

---

## grpc-core (Priority: Critical)

### status.rs (~60-70%)

| ID | Branch/Path | Line(s) | Description |
|----|-------------|---------|-------------|
| C1 | `from_header_map` missing grpc-status | 227 | Returns None when header absent — no test |
| C2 | `from_header_map` invalid UTF-8 in message | 230-234 | Percent-decode failure on corrupted message |
| C3 | `from_header_map` invalid base64 details | 239-245 | Returns Internal status — needs explicit test |
| C4 | `add_header` invalid HeaderValue | 293, 303 | Error creating header from encoded bytes |
| C5 | `into_http` fallback on add_header error | 326-330 | Sends only status code when message fails |
| C6 | `From<io::Error>` unmapped variants | 370-394 | BrokenPipe, WouldBlock, AlreadyExists, etc. fall to Unknown |
| C7 | `set_source` and Error::source() | 191-194 | Source chain storage/retrieval never tested |
| C8 | `infer_grpc_status` unmapped HTTP codes | 436-447 | Non-standard HTTP status → Code::Unknown |

### codec/decode.rs (~50% — Most Critical)

| ID | Branch/Path | Line(s) | Description |
|----|-------------|---------|-------------|
| C9 | ReadHeader insufficient bytes | 175-178 | Partial header in buffer |
| C10 | Compression flag=0 (uncompressed) | 180-183 | Flag parsing for no-compression case |
| C11 | Compression flag=1 with encoding | 184-193 | Decompress path in decode_chunk |
| C12 | Invalid compression flag (>=2) | 180-197 | Error on bad flag byte |
| C13 | Message size limit exceeded | 199-207 | decode_chunk rejects oversized message |
| C14 | ReadBody insufficient data | 213-228 | Partial message in buffer |
| C15 | Cancelled error on Request direction | 240-241 | Stream error mapped to Cancelled |
| C16 | Body EOF with remaining buffer | 249-254 | Incomplete message at stream end |
| C17 | Trailer frame handling | 258-270 | Data frame with trailers merge |
| C18 | `infer_grpc_status` in response() | 273-281 | Status inference from trailers |

### codec/encode.rs (~75%)

| ID | Branch/Path | Line(s) | Description |
|----|-------------|---------|-------------|
| C19 | poll_next Pending with buffered data | 76-77 | Flush buffer when stream is not ready |
| C20 | finish_encoding size > max_message_size | 159-186 | OutOfRange error for oversized message |
| C21 | EncodeBody error → trailers (Server) | 276-295 | Error converted to gRPC trailers |
| C22 | EncodeState::trailers Client variant | 248-261 | Client never sends trailers |

### Other grpc-core files

| ID | File | Branch/Path | Line(s) | Description |
|----|------|-------------|---------|-------------|
| C23 | request.rs | `into_http` sanitize=true | 84-102 | Reserved header stripping |
| C24 | request.rs | timeout edge cases (all units) | 174-201 | n/u/m/S/M/H unit boundaries |
| C25 | body.rs | `Body::new` is_end_stream early return | 33-34 | Empty body optimization |
| C26 | body.rs | poll_frame Wrap variant | 55-63 | Delegate to inner body |
| C27 | metadata.rs | merge with overlapping keys | 89-92 | Overwrite vs extend behavior |
| C28 | codec/buffer.rs | DecodeBuf advance past len | 37-41 | Assertion panic |
| C29 | prost_codec.rs | Encode error (insufficient space) | 61-65 | Maps to Status::internal |
| C30 | prost_codec.rs | Decode error (malformed bytes) | 83-88 | Maps to Status::internal |
| C31 | compression.rs | from_encoding_header rejection | 54-78 | Unsupported/unknown encoding error |
| C32 | compression.rs | accept-encoding multi-value | 81-98 | First disabled, second enabled |

---

## grpc-server (Priority: Critical)

| ID | Branch/Path | File:Line(s) | Description |
|----|-------------|-------------|-------------|
| S1 | Timeout expiration in HyperServiceWrapper | server.rs:379-386 | DeadlineExceeded path never triggered |
| S2 | Concurrency limit exhausted (Err branch) | server.rs:255-262 | Semaphore wait path |
| S3 | TLS handshake success | server.rs:274-276 | Feature-gated, never tested |
| S4 | TLS handshake error | server.rs:278-280 | Feature-gated, never tested |
| S5 | HTTP/2 config application | server.rs:325-345 | All 7 config branches |
| S6 | serve_connection error | server.rs:348-350 | Connection-level error logging |
| S7 | Compression encoding rejection | grpc.rs:186-189 | Unsupported encoding in request |
| S8 | Empty request message | grpc.rs:200-203 | Stream yields None |
| S9 | Service error response (t! macro) | grpc.rs:249 | Err path in map_response |
| S10 | server_streaming pattern | grpc.rs:106-129 | Only unary tested |
| S11 | client_streaming pattern | grpc.rs:132-154 | Only unary tested |
| S12 | streaming (bidi) pattern | grpc.rs:157-175 | Only unary tested |
| S13 | Size limit enforcement | grpc.rs:64-74 | max_decoding/encoding_message_size |
| S14 | into_layered | router.rs:116-121 | Never called in tests |
| S15 | Interceptor metadata modification | interceptor.rs:87-95 | Only read/reject tested, not modify |

---

## grpc-client (Priority: Critical)

| ID | Branch/Path | File:Line(s) | Description |
|----|-------------|-------------|-------------|
| CL1 | Channel::call with timeout | channel.rs:222-232 | DeadlineExceeded on timeout |
| CL2 | Channel::call actual request flow | channel.rs:200-220 | No test sends real HTTP request |
| CL3 | apply_h2_config all branches | channel.rs:37-56 | All 6 config fields |
| CL4 | All TLS connect methods | channel.rs:94-168 | Feature-gated, zero tests |
| CL5 | Endpoint connect_timeout enforcement | endpoint.rs:158-166 | tokio::time::timeout path |
| CL6 | Endpoint TLS mode dispatch | endpoint.rs:139-156 | Feature-gated, zero tests |
| CL7 | Grpc::ready() | grpc.rs:104-109 | Never called in tests |
| CL8 | Grpc::unary() | grpc.rs:112-128 | Core RPC pattern untested |
| CL9 | Grpc::client_streaming() | grpc.rs:131-165 | Core RPC pattern untested |
| CL10 | Grpc::server_streaming() | grpc.rs:168-184 | Core RPC pattern untested |
| CL11 | Grpc::streaming() core impl | grpc.rs:189-225 | The main method — zero tests |
| CL12 | prepare_request path merging | grpc.rs:234-245 | Origin + path concatenation |
| CL13 | create_response all branches | grpc.rs:282-324 | Trailers-only, compression, status |
| CL14 | BalancedChannel::call round-robin | balance.rs:70-74 | Never sends actual requests |

---

## grpc-build (Priority: High)

| ID | Branch/Path | File:Line(s) | Description |
|----|-------------|-------------|-------------|
| B1 | compile_protos OUT_DIR missing | protobuf.rs:35-36 | Error propagation |
| B2 | compile_protos file read failure | protobuf.rs:43-48 | Non-existent proto file |
| B3 | compile_protos analysis failure | protobuf.rs:57-62 | Invalid proto syntax |
| B4 | compile_protos codegen failure | protobuf.rs:65-70 | Codegen error |
| B5 | compile_fbs OUT_DIR missing | flatbuffers.rs:32-33 | Error propagation |
| B6 | compile_fbs compilation failure | flatbuffers.rs:40-41 | Invalid .fbs file |
| B7 | compile_fbs fallback filename | flatbuffers.rs:52-56 | No file stem → "flatbuffers_generated" |

---

## grpc-web (Priority: High)

| ID | Branch/Path | File:Line(s) | Description |
|----|-------------|-------------|-------------|
| W1 | poll_decode partial base64 buffering | call.rs:115-129 | 4-byte alignment retry |
| W2 | poll_decode re-wake on insufficient data | call.rs:127-129 | Pending when no aligned data |
| W3 | poll_encode trailer capture | call.rs:181-185 | Frame with trailers_ref() |
| W4 | poll_encode end-of-stream trailers | call.rs:191-200 | Empty trailer frame on body end |
| W5 | Service non-POST request | service.rs:50-54 | Only POST allowed for gRPC-Web |
| W6 | Service missing content-type | service.rs:50-54 | Default empty string handling |
| W7 | Base64 encoding of multiple data frames | call.rs:174-180 | Sequential frame encoding |

---

## grpc-reflection (Priority: Medium)

| ID | Branch/Path | File:Line(s) | Description |
|----|-------------|-------------|-------------|
| R1 | Invalid FileDescriptorSet decode | lib.rs:102-105 | Corrupted descriptor bytes |
| R2 | File re-encode error | lib.rs:110-114 | Inner encode failure |
| R3 | None message_request | lib.rs:240-281 | Missing oneof variant |
| R4 | AllExtensionNumbersOfType | lib.rs:270-275 | Unimplemented error response |
| R5 | Unknown path in Service::call | lib.rs:310 | Fallback for non-reflection paths |
| R6 | Deeply nested types (3+ levels) | lib.rs:62-75 | Only 2-level tested |

---

## grpc-types (Priority: Medium)

| ID | Branch/Path | File:Line(s) | Description |
|----|-------------|-------------|-------------|
| T1 | error_details decode failure | status_ext.rs:93 | Malformed details bytes |
| T2 | get_details_* wrong variant | status_ext.rs:97-195 | Detail present but different type |
| T3 | FromAny decode corrupted value | any_ext.rs:91 | Truncated/invalid Any.value |

---

## grpc-codegen (Priority: Medium)

| ID | Branch/Path | File:Line(s) | Description |
|----|-------------|-------------|-------------|
| G1 | comments_to_doc_tokens | ir.rs:97-106 | Function never tested directly |
| G2 | Invalid response_index only | flatbuffers.rs:57-67 | Only request_index error tested |
| G3 | TokenStream parse panics | client_gen.rs:88-100, server_gen.rs:243-254 | Invalid codec/type paths |

---

## grpc-health / grpc-codec-flatbuffers (Priority: Low)

| ID | Branch/Path | File:Line(s) | Description |
|----|-------------|-------------|-------------|
| H1 | Unknown path in HealthServer::call | health/lib.rs:231-234 | Fallback for non-health paths |
| H2 | Concurrent Watch channel creation | health/lib.rs:183-193 | Race condition on same service |
| F1 | FlatBuffersDecoder empty buffer | codec-flatbuffers/lib.rs:101-107 | remaining() == 0 |
| F2 | FlatBuffersDecoder decode error | codec-flatbuffers/lib.rs:104-105 | Error mapping verification |

---

## Recommended Fix Order

### Phase 1 — Core (highest impact, ~50 tests)
1. **C9-C18**: codec/decode.rs state machine — most complex, most untested
2. **C19-C22**: codec/encode.rs error/size paths
3. **C1-C8**: status.rs error paths and edge cases
4. **C29-C32**: prost_codec and compression edge cases

### Phase 2 — Server + Client RPC paths (~40 tests)
5. **S1, S7-S13**: Server RPC patterns and error paths
6. **CL1, CL7-CL13**: Client RPC patterns and response handling
7. **S2, S5-S6**: Server connection management
8. **CL3, CL5, CL14**: Client connection and load balancing

### Phase 3 — Supporting crates (~30 tests)
9. **B1-B7**: grpc-build error paths
10. **W1-W7**: grpc-web protocol translation
11. **R1-R6**: grpc-reflection edge cases
12. **T1-T3, G1-G3**: grpc-types and grpc-codegen gaps

### Phase 4 — Remaining (~15 tests)
13. **H1-H2, F1-F2**: Health and FlatBuffers edge cases
14. **S3-S4, CL4, CL6**: TLS paths (feature-gated)
15. **C23-C28**: Minor grpc-core gaps
