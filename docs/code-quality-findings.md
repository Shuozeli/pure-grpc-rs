# Code Quality Findings

Audit performed 2026-03-26. Codebase: pure-grpc-rs (workspace with 10 crates + 2 examples).
Vendor directory (`vendor/tonic/`) excluded -- reference only, not project code.

## 1. Duplication

### 1.1 Http2Config duplicated between server and client -- DONE
- **Location:** `grpc-server/src/server.rs:33-41` (`Http2Config`)
- **Also at:** `grpc-client/src/endpoint.rs:20-27` (`Http2Config`)
- **Problem:** Two identical structs with the same fields (`initial_stream_window_size`, `initial_connection_window_size`, `adaptive_window`, `max_frame_size`, `keep_alive_interval`, `keep_alive_timeout`). The server version also has `max_concurrent_streams` (server-only). Both are used for the same purpose -- applying HTTP/2 settings. Changes to one must be manually mirrored.
- **Fix:** Extract a shared `Http2Config` struct into `grpc-core` (which both crates depend on). Keep the server-only `max_concurrent_streams` as a separate field or extension on the server side.
- **Resolution:** Created `grpc-core/src/http2_config.rs` with shared `Http2Config`. Server uses `ServerHttp2Config` that composes the shared config with the server-only `max_concurrent_streams`. Client imports directly from `grpc_core::Http2Config`.

### 1.2 BoxError type alias redefined in three client modules -- DONE
- **Location:** `grpc-client/src/endpoint.rs:5`
- **Also at:** `grpc-client/src/balance.rs:13`, `grpc-client/src/channel.rs:10`
- **Problem:** `type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>` is defined identically in three files within the same crate. `grpc-core` already exports `BoxError` as a public type.
- **Fix:** Remove the three local aliases and use `grpc_core::BoxError` instead, or define it once at the `grpc-client` crate root and import from there.
- **Resolution:** Replaced all three local aliases with `use grpc_core::BoxError`.

### 1.3 TestCodec/TestEncoder/TestDecoder copy-pasted across test modules -- DONE
- **Location:** `grpc-server/src/grpc.rs:288-335` (TestCodec, TestEncoder, TestDecoder)
- **Also at:** `grpc-client/src/grpc.rs:428-479` (identical TestCodec, TestEncoder, TestDecoder)
- **Also at:** `grpc-core/src/codec/encode.rs:303-313` (TestEncoder only)
- **Problem:** Three near-identical test helper types are independently defined in three test modules. A `Vec<u8>` codec with put_slice encoding and copy_to_bytes decoding is repeated verbatim. Any change to the test codec API must be updated in all three places.
- **Fix:** Create a `#[cfg(test)]` test utilities module in `grpc-core` (e.g., `grpc-core/src/codec/test_helpers.rs`) that exports `TestCodec`, `TestEncoder`, and `TestDecoder`. The other crates can use them via `#[cfg(test)] use grpc_core::codec::test_helpers::*`.
- **Resolution:** Created `grpc-core/src/codec/test_helpers.rs` behind `#[cfg(any(test, feature = "test-helpers"))]`. Added `test-helpers` feature to grpc-core. Updated grpc-server, grpc-client, and grpc-core's encode.rs to import from the shared module.

### 1.4 get_details_* methods are 10 copy-pasted blocks -- DONE
- **Location:** `grpc-types/src/status_ext.rs:97-195`
- **Problem:** Ten `get_details_*` methods on `StatusExt` are structurally identical -- each calls `self.error_details().ok()?.into_iter().find_map(|d| match d { ErrorDetail::Variant(v) => Some(v), _ => None })`. Only the variant name changes. This is 100 lines of boilerplate.
- **Fix:** Define a macro:
  ```rust
  macro_rules! impl_get_details {
      ($fn_name:ident, $variant:ident, $type:ty) => {
          fn $fn_name(&self) -> Option<$type> {
              self.error_details().ok()?.into_iter().find_map(|d| match d {
                  ErrorDetail::$variant(v) => Some(v),
                  _ => None,
              })
          }
      };
  }
  ```
  Then use it 10 times in the impl block. This reduces the code to ~30 lines.
- **Resolution:** Implemented the `impl_get_details!` macro and replaced all 10 methods with macro invocations.

### 1.5 Compression test structure duplicated across gzip/deflate/zstd -- SKIPPED
- **Location:** `grpc-core/src/codec/compression.rs:246-382` (gzip_tests, deflate_tests, zstd_tests modules)
- **Problem:** Three test modules are structurally identical -- each has `compress_decompress_roundtrip`, `encoding_from_str`, `enabled_encodings`, and `accept_encoding_header_value` tests that differ only in the `CompressionEncoding` variant name and test data strings.
- **Fix:** Use a macro to generate all four tests parameterized by the encoding variant. This eliminates ~80 lines of duplication. *(Low priority -- test duplication is tolerable.)*
- **Skipped:** Low priority. Test duplication is tolerable and the individual test modules are easier to read/debug than macro-generated tests.

## 2. Unsafe Patterns

### 2.1 `expect()` in non-test code: request timeout encoding -- DONE
- **Location:** `grpc-core/src/request.rs:119`
- **Problem:** `duration_to_grpc_timeout` is called, and its result is parsed with `.parse().expect(...)`. While the comment explains why it is safe ("always valid ASCII"), the function at line 200 also uses `.expect("duration is unrealistically large")`. An extremely large `Duration` (e.g., `Duration::MAX`) will panic at runtime instead of returning an error.
- **Fix:** Change `set_timeout` to return `Result<(), Status>` and propagate the error, or clamp the duration to the gRPC max (99,999,999 hours).
- **Resolution:** Clamped the duration to the gRPC max (99,999,999 hours) before formatting. After clamping, the hours format always succeeds, so the `expect` is unreachable but kept with an updated message for safety.

### 2.2 `expect()` in non-test code: compression encoding header -- SKIPPED
- **Location:** `grpc-core/src/codec/compression.rs:149`
- **Problem:** `EnabledCompressionEncodings::into_accept_encoding_header_value` calls `.expect("encoding names are valid ASCII")`. While the encoding names are indeed ASCII constants, `expect()` in library code is a panicking API that callers cannot recover from.
- **Fix:** Return `Option<Result<HeaderValue, ...>>` or keep the `expect()` but add a `// SAFETY:` comment explaining the invariant. *(Low priority -- the invariant is sound.)*
- **Skipped:** Low priority. The encoding names are compile-time constants guaranteed to be ASCII. The invariant is sound.

### 2.3 `expect()` in non-test code: semaphore acquire -- DONE
- **Location:** `grpc-server/src/server.rs:260`
- **Problem:** `.expect("semaphore should not be closed")` -- if the `Semaphore` were ever closed (e.g., dropped early due to a bug), this panics inside the accept loop, killing the server.
- **Fix:** Replace with a match that logs a warning and continues or breaks the loop gracefully.
- **Resolution:** Replaced with `match` that logs a warning via `tracing::warn!` and breaks the accept loop gracefully.

### 2.4 Unsafe block for `advance_mut` in encode -- SKIPPED
- **Location:** `grpc-core/src/codec/encode.rs:124-126`
- **Problem:** `unsafe { buf.advance_mut(HEADER_SIZE); }` -- the safety comment is adequate and the reservation happens on line 120, so this is correct. However, it could be made safe by writing placeholder bytes (e.g., `buf.put_bytes(0, HEADER_SIZE)`) and then overwriting them in `finish_encoding`. The performance difference is negligible.
- **Fix:** Consider replacing with safe `buf.put_bytes(0, HEADER_SIZE)` to eliminate the unsafe block entirely. *(Low priority -- current code is correct.)*
- **Skipped:** Low priority. The unsafe block is correct, has an adequate safety comment, and the reservation is verified. The performance difference of writing placeholder bytes is negligible but the existing code is idiomatic.

## 3. Missing Abstractions

### 3.1 base64 engine construction repeated -- DONE
- **Location:** `grpc-core/src/status.rs:458-476`
- **Problem:** `base64_engine()` and `base64_no_pad_engine()` construct `GeneralPurpose` engines from scratch on every call. While the compiler may optimize this, these are non-trivial constructors.
- **Fix:** Use `static` or `LazyLock` to create them once:
  ```rust
  use std::sync::LazyLock;
  static BASE64_ENGINE: LazyLock<GeneralPurpose> = LazyLock::new(|| { ... });
  ```
- **Resolution:** Replaced `base64_engine()` and `base64_no_pad_engine()` functions with `LazyLock` statics `BASE64_ENGINE` and `BASE64_NO_PAD_ENGINE`.

### 3.2 `ErrorDetail::with_error_details` match arm is copy of `ErrorDetail` enum -- DONE
- **Location:** `grpc-types/src/status_ext.rs:62-74`
- **Problem:** The match on `ErrorDetail` to call `.into_any()` has 11 arms that mirror the enum variants. Adding a new error detail type requires updating both the enum, the `impl_any_traits!` macro invocations, the `decode_any` dispatch, AND this match. Four coordination points for one addition.
- **Fix:** Add a method `fn into_any(self) -> prost_types::Any` directly on `ErrorDetail` (since every variant already has `IntoAny`), then the match in `with_error_details` becomes just `.map(|d| d.into_any())`.
- **Resolution:** Added `ErrorDetail::into_any()` method in `any_ext.rs`. Simplified `with_error_details` to `.map(|d| d.into_any())`. Removed unused `IntoAny` import from `status_ext.rs`.

## 4. Silent Failures

### 4.1 Status clone in decode error path loses source chain -- SKIPPED
- **Location:** `grpc-core/src/codec/decode.rs:243`
- **Problem:** `status.clone()` -- as noted by the existing `// TODO(refactor):` comment, this clones the status to both store it in the error state and return it. Because `Status` wraps an `Arc<dyn Error>` source, the clone is cheap, but semantically the stored status and the returned status are independent copies. If a caller modifies the returned status's metadata, the stored copy is stale.
- **Fix:** As the TODO suggests, either use `Arc<Status>` for zero-copy sharing, or split the error path to avoid needing two copies.
- **Skipped:** The existing `// TODO(refactor):` comment already tracks this. The clone is cheap (Arc-backed) and the semantic issue is theoretical -- callers don't modify returned statuses. Will address when refactoring the decode error path.

### 4.2 `to_str().unwrap_or("")` silently swallows invalid header values -- DONE
- **Location:** `grpc-core/src/codec/compression.rs:62`
- **Problem:** `val.to_str().unwrap_or("")` -- if the `grpc-encoding` header contains non-ASCII bytes, this silently treats it as an empty string, which maps to `None` (no compression). The caller gets no indication that the header was malformed.
- **Fix:** Return an error status for non-UTF8 encoding headers:
  ```rust
  let encoding_str = val.to_str().map_err(|_| Status::internal("grpc-encoding header is not valid UTF-8"))?;
  ```
- **Resolution:** Replaced `unwrap_or("")` with `.map_err(|_| Status::internal("grpc-encoding header is not valid UTF-8"))?`.

## 5. `#[allow(...)]` Suppressing Real Warnings

### 5.1 Broad `#[allow(clippy::all, ...)]` on generated types module -- SKIPPED
- **Location:** `grpc-types/src/lib.rs:54`
- **Problem:** `#[allow(clippy::all, non_camel_case_types, missing_docs)]` is applied to the `google` module. While this is standard practice for generated code (the `include!` on line 79 pulls in protoc-generated code), the module also contains hand-written re-export paths and a shadow module. The `clippy::all` suppression could mask real issues in the hand-written code.
- **Fix:** Move the `#[allow]` to be more targeted -- apply it only to the `include!` line or the generated submodule, not the entire `google` module. *(Low priority -- the hand-written code is minimal.)*
- **Skipped:** Low priority. The hand-written code in the module is minimal (just re-exports). The `#[allow]` on generated code is standard practice.

### 5.2 `#[allow(...)]` on examples -- NO ACTION NEEDED
- **Location:** `examples/greeter-fbs/src/lib.rs:5`
- **Problem:** Broad allow attributes on generated FlatBuffers code. This is acceptable for generated code.
- **Fix:** No action needed -- this is standard practice for generated code.

## 6. Dead Code / Unused Patterns

### 6.1 `_source` variable in `compile_protos` is unused -- DONE
- **Location:** `grpc-build/src/protobuf.rs:43`
- **Problem:** `let _source = std::fs::read_to_string(proto_path)...` -- the file is read but its contents are never used. The underscore prefix suppresses the warning. The file is read only to verify it exists and is readable, but `analyze_files` will also read it via the `DirResolver`. This is a redundant read.
- **Fix:** Replace with `std::fs::metadata(proto_path).map_err(...)` to check existence without reading the file, or remove the check entirely since `analyze_files` will produce a clear error if the file is missing.
- **Resolution:** Replaced `std::fs::read_to_string` with `std::fs::metadata` to check existence without reading file contents.

### 6.2 `response()` method's `Ok` case maps to `Err(None)` confusingly -- SKIPPED
- **Location:** `grpc-core/src/status.rs:445`
- **Problem:** In `infer_grpc_status`, when no trailers are present and the HTTP status is 200 OK, the function returns `Err(None)`. This is semantically confusing -- a 200 OK with no gRPC status is treated as an error case (returning `Err`), and `None` means "no specific gRPC status could be inferred." The `Result<(), Option<Status>>` return type makes the logic hard to follow.
- **Fix:** Consider a dedicated enum return type:
  ```rust
  enum GrpcStatusInference { Ok, Error(Status), MissingStatus }
  ```
  *(Low priority -- the current code works correctly, it's just hard to read.)*
- **Skipped:** Low priority. The function works correctly. Changing the return type would require updating all callers for a readability-only improvement.

## 7. Noise / Documentation

### 7.1 Duplicate doc comment on `service_from_proto` -- DONE
- **Location:** `grpc-codegen/src/protobuf.rs:13-21`
- **Problem:** The function has two `///` doc blocks -- the first says "Convert a protobuf-rs `ServiceDescriptorProto` into a `ServiceDef`" (lines 13-14), and the second says the same thing with additional detail (lines 16-21). Rust will render both, creating duplicate opening sentences.
- **Fix:** Remove lines 13-14 (the shorter doc block). Keep only the detailed version starting at line 16.
- **Resolution:** Merged the two doc blocks into one, removing the duplicate opening sentence.

## 8. Potential Improvements (Not Bugs)

### 8.1 `into_parts` signature inconsistency between Request and Response -- SKIPPED
- **Location:** `grpc-core/src/request.rs:55` vs `grpc-core/src/response.rs:41`
- **Problem:** `Request::into_parts` returns `(MetadataMap, Extensions, T)` while `Response::into_parts` returns `(MetadataMap, T, Extensions)`. The order of `T` and `Extensions` is swapped. This is a footgun for anyone writing generic code.
- **Fix:** Standardize both to the same order. Since this is a public API, this would be a breaking change. Add a deprecation note if not addressed before 1.0.
- **Skipped:** Breaking change to public API. Should be addressed before 1.0 release but not as part of this quality pass.

### 8.2 `Endpoint::connect_timeout` is accepted but has no effect on plaintext connections -- SKIPPED
- **Location:** `grpc-client/src/endpoint.rs:158-166`
- **Problem:** `connect_timeout` wraps the `connect_fut` in `tokio::time::timeout`. However, for plaintext HTTP/2 via `hyper_util`, `Channel::connect_with_h2_config` returns immediately because the hyper client uses lazy connection establishment. The timeout only applies to the `Channel` construction, not actual TCP connection establishment. For TLS, the timeout is more meaningful because TLS handshake happens during `connect`.
- **Fix:** Document this behavior explicitly in the `connect_timeout` doc comment, or implement eager connection establishment for plaintext channels.
- **Skipped:** Requires design decision on whether to implement eager connection establishment. Out of scope for quality pass.

### 8.3 `BalancedChannel::from_uris` panics on empty input -- DONE
- **Location:** `grpc-client/src/balance.rs:44`
- **Problem:** `assert!(!endpoints.is_empty(), ...)` panics on empty input. For a library, returning `Result::Err` would be more user-friendly.
- **Fix:** Return `Err(BoxError)` instead of panicking.
- **Resolution:** Replaced `assert!` with an early `return Err(...)`. Updated doc comments to reflect the error return.

### 8.4 Server `concurrency_limit` panics on zero -- SKIPPED
- **Location:** `grpc-server/src/server.rs:78`
- **Problem:** `assert!(limit > 0, ...)` panics on zero. Same issue as 8.3.
- **Fix:** Return `Result` or use a newtype that enforces the invariant at the type level.
- **Skipped:** The server builder uses a fluent API pattern where all methods return `Self`. Changing `concurrency_limit` to return `Result` would break the chain pattern. The `assert!` in a builder is acceptable -- it catches programmer errors at construction time, not runtime user input.
