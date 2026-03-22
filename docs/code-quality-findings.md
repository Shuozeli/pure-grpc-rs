# Code Quality Findings -- pure-grpc-rs

Audit performed 2026-03-20, 4-phase (Analyze, Document, Fix, Verify).

## Summary

| ID   | Category             | Severity | File                                   | Line(s) | Status |
|------|----------------------|----------|----------------------------------------|---------|--------|
| F1   | Unsafe `.unwrap()`   | Medium   | grpc-core/src/request.rs               | 117     | FIXED  |
| F2   | Unsafe `.expect()`   | Medium   | grpc-core/src/codec/prost_codec.rs     | 63      | FIXED  |
| F3   | Duplicated test code | Low      | grpc-codegen/src/{ir,server_gen,client_gen}.rs | various | WONTFIX |
| F4   | Duplicated test code | Low      | grpc-server/src/{router,server,interceptor}.rs | various | WONTFIX |
| F5   | `#[allow(...)]`      | Medium   | examples/greeter-fbs/src/lib.rs        | 5       | FIXED  |
| F6   | Confusing Debug impl | Low      | grpc-client/src/channel.rs             | 177-189 | FIXED  |
| F7   | Unnecessary clone    | Low      | grpc-health/src/lib.rs                 | 118-119 | FIXED  |
| F8   | Comment restating code | Low    | grpc-health/src/lib.rs                 | 206     | FIXED  |

---

## Detailed Findings

### F1 -- Unsafe `.unwrap()` in `set_timeout` (Medium)

**File:** `grpc-core/src/request.rs:117`
**Problem:** `duration_to_grpc_timeout(deadline).parse().unwrap()` panics in production if the generated timeout string somehow fails to parse as a `HeaderValue`. While `duration_to_grpc_timeout` always produces valid strings, `.unwrap()` in non-test code violates fail-fast-with-error-propagation.
**Fix:** Replace `.unwrap()` with `.expect("grpc-timeout value is always valid ASCII digits + unit char")` to document the invariant.

### F2 -- Unsafe `.expect()` in ProstEncoder (Medium)

**File:** `grpc-core/src/codec/prost_codec.rs:62-63`
**Problem:** `item.encode(buf).expect("Message only errors if not enough space")` panics at runtime. `EncodeBuf` grows dynamically so this should never trigger, but panicking in a codec encode path is wrong -- the caller expects `Result`. The error should be propagated.
**Fix:** Replace `.expect(...)` with `.map_err(|e| Status::internal(format!("prost encode error: {e}")))?`.

### F3 -- Duplicated `sample_service()` in codegen tests (Low)

**File:** `grpc-codegen/src/ir.rs:112`, `server_gen.rs:279`, `client_gen.rs:190`
**Problem:** Three nearly-identical `sample_service()` test helper functions. They define the same `ServiceDef` with minor variations.
**Status:** WONTFIX. These are in `#[cfg(test)]` modules which cannot share private functions across files without refactoring to a shared test module. The duplication is contained and each copy is small. Moving them to `test_util.rs` would add coupling for minimal benefit.

### F4 -- Duplicated `MockService` in server tests (Low)

**File:** `grpc-server/src/router.rs:123`, `server.rs:327`, `interceptor.rs:114`
**Problem:** Three `MockService` structs implementing `tower::Service` for tests. Each is slightly different (router's has a name field, others don't).
**Status:** WONTFIX. Same reasoning as F3 -- different enough to justify separate definitions, and contained in test modules.

### F5 -- `#[allow(...)]` suppressing real warnings (Medium)

**File:** `examples/greeter-fbs/src/lib.rs:5`
**Problem:** `#[allow(unused_imports, dead_code, non_snake_case, clippy::all)]` is too broad. The CLAUDE.md rule says "do NOT use `#[allow(...)]` to bypass clippy or rustdoc warnings -- fix the root cause." This blanket allow could hide real issues in the generated code. A narrower scope is appropriate since the generated FlatBuffers code does produce these warnings.
**Fix:** Keep the allow attributes but scope them more precisely: keep `non_snake_case` (FlatBuffers naming), `dead_code` and `unused_imports` (not all generated types are used), but remove `clippy::all` and replace with specific clippy lints that the generated code triggers.

### F6 -- Confusing Debug impl for Channel (Low)

**File:** `grpc-client/src/channel.rs:177-189`
**Problem:** The TLS debug field uses a confusing double-negation pattern: `matches!(self.inner, ChannelInner::Http(_)).then_some("no").unwrap_or("yes")`. While logically correct, it reads as "if Http then no else yes" which requires careful mental parsing.
**Fix:** Simplify to a direct `if`/`else` or `match`.

### F7 -- Unnecessary clone of `state` (Low)

**File:** `grpc-health/src/lib.rs:118-119`
**Problem:** `state: state.clone()` is used for `server` construction when `state` (an `Arc`) is already cloned for `handle` on line 116. The final use of `state` on line 119 could use `state` directly (move it) instead of cloning, since `state` is not used after this point.
**Fix:** Use `state` directly (move) for the last usage.

### F8 -- Comment restating code (Low)

**File:** `grpc-health/src/lib.rs:206`
**Problem:** `// Need the map import for WatchStream` is a comment that restates what the `use` statement does. The import itself (`StreamExt as _`) is self-explanatory.
**Fix:** Remove the comment.
