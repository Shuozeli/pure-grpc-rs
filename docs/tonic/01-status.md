# Status & Code

**Source:** `tonic/src/status.rs` (1,092 lines)
**Dependencies:** `bytes`, `http`, `percent-encoding`, `base64`, `tracing`
**Conditionally:** `h2` (feature = "server"), `hyper` (feature = "server"|"channel")

## Overview

`Status` is the gRPC error type. It maps to the three gRPC trailing headers:
`grpc-status`, `grpc-message`, `grpc-status-details-bin`. Tonic boxes the inner
struct for size optimization — keeping `Result<T, Status>` small.

## Type Definitions

```rust
#[derive(Clone)]
pub struct Status(Box<StatusInner>);

#[derive(Clone)]
struct StatusInner {
    code: Code,
    message: String,
    details: Bytes,            // binary, opaque — used by richer error model
    metadata: MetadataMap,     // custom trailing metadata
    source: Option<Arc<dyn Error + Send + Sync + 'static>>,
}
```

**Design decision: boxing.** `Status` wraps `StatusInner` in a `Box` so that
`Result<Response<T>, Status>` stays small (1 pointer). This matters because
`Status` appears in nearly every return type.

## Code Enum

17 variants matching the [gRPC spec](https://github.com/grpc/grpc/blob/master/doc/statuscodes.md):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Code {
    Ok = 0,              Cancelled = 1,        Unknown = 2,
    InvalidArgument = 3, DeadlineExceeded = 4,  NotFound = 5,
    AlreadyExists = 6,   PermissionDenied = 7,  ResourceExhausted = 8,
    FailedPrecondition = 9, Aborted = 10,       OutOfRange = 11,
    Unimplemented = 12,  Internal = 13,         Unavailable = 14,
    DataLoss = 15,       Unauthenticated = 16,
}
```

Conversions:
- `Code::from_i32(i32)` — const fn, unknown values map to `Code::Unknown`
- `Code::from_bytes(&[u8])` — optimized byte parsing (1 or 2 byte match)
- `Code::to_header_value()` — returns `HeaderValue::from_static`
- `From<i32>` / `Into<i32>` impls

## Constructors

Named constructors for every code:
```rust
Status::new(Code, impl Into<String>)
Status::ok(msg), Status::cancelled(msg), Status::unknown(msg), ...
Status::with_details(Code, msg, Bytes)
Status::with_metadata(Code, msg, MetadataMap)
Status::with_details_and_metadata(Code, msg, Bytes, MetadataMap)
```

## HTTP Header Serialization

Three gRPC-reserved headers:
```rust
const GRPC_STATUS: HeaderName = "grpc-status";
const GRPC_MESSAGE: HeaderName = "grpc-message";
const GRPC_STATUS_DETAILS: HeaderName = "grpc-status-details-bin";
```

**Encoding (`add_header` / `to_header_map`):**
1. Insert all custom metadata headers (sanitized — reserved names stripped)
2. `grpc-status` = code as decimal string
3. `grpc-message` = percent-encoded message (CONTROLS + space, `"#%<>\`?{}`)
4. `grpc-status-details-bin` = base64-encoded (standard, no padding) details bytes

**Decoding (`from_header_map`):**
1. Parse `grpc-status` via `Code::from_bytes`
2. Percent-decode `grpc-message` (UTF-8)
3. Base64-decode `grpc-status-details-bin`
4. Remaining headers become `MetadataMap`

## Percent-Encoding Set

```rust
const ENCODING_SET: &AsciiSet = &CONTROLS
    .add(b' ').add(b'"').add(b'#').add(b'%')
    .add(b'<').add(b'>').add(b'`').add(b'?').add(b'{').add(b'}');
```

Per [gRPC HTTP/2 spec](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md),
`grpc-message` is percent-encoded.

## Base64 Configuration

Two engine configs in `util.rs`:
- `STANDARD` — with padding, indifferent decode (used for decoding details)
- `STANDARD_NO_PAD` — no padding, indifferent decode (used for encoding details)

## Error Conversions

### `From<std::io::Error>`
Maps `ErrorKind` to `Code`:
| ErrorKind | Code |
|-----------|------|
| BrokenPipe, WouldBlock, WriteZero, Interrupted | Internal |
| ConnectionRefused, ConnectionReset, NotConnected, AddrInUse, AddrNotAvailable | Unavailable |
| AlreadyExists | AlreadyExists |
| ConnectionAborted | Aborted |
| InvalidData | DataLoss |
| InvalidInput | InvalidArgument |
| NotFound | NotFound |
| PermissionDenied | PermissionDenied |
| TimedOut | DeadlineExceeded |
| UnexpectedEof | OutOfRange |
| _ | Unknown |

### `From<h2::Error>` (feature = "server")
Maps `h2::Reason` to `Code`:
| h2::Reason | Code |
|------------|------|
| NO_ERROR, PROTOCOL_ERROR, INTERNAL_ERROR, FLOW_CONTROL_ERROR, SETTINGS_TIMEOUT, COMPRESSION_ERROR, CONNECT_ERROR | Internal |
| REFUSED_STREAM | Unavailable |
| CANCEL | Cancelled |
| ENHANCE_YOUR_CALM | ResourceExhausted |
| INADEQUATE_SECURITY | PermissionDenied |
| _ | Unknown |

### `from_hyper_error` (feature = "server"|"channel")
- `is_timeout()` -> Unavailable (keep-alive ping expiry)
- `is_canceled()` -> Cancelled
- Source chain: tries h2::Error mapping

### `from_error(Box<dyn Error>)` — Source Chain Walk
`find_status_in_source_chain` walks `Error::source()` chain looking for:
1. `Status` — clone code/message/details/metadata (not source)
2. `TimeoutExpired` — map to Cancelled
3. `ConnectError` — map to Unavailable
4. `hyper::Error` — delegate to `from_hyper_error`

If nothing found, returns `Code::Unknown` with error message.

### Reverse: `Status -> h2::Error`
- Cancelled -> `h2::Reason::CANCEL`
- Everything else -> `h2::Reason::INTERNAL_ERROR`

## HTTP Status Code Mapping

`infer_grpc_status(trailers, status_code)` — used when trailers are missing:

| HTTP Status | gRPC Code |
|-------------|-----------|
| 400 Bad Request | Internal |
| 401 Unauthorized | Unauthenticated |
| 403 Forbidden | PermissionDenied |
| 404 Not Found | Unimplemented |
| 429, 502, 503, 504 | Unavailable |
| 200 (no trailers) | returns `Err(None)` — special case |
| _ | Unknown |

## Sentinel Error Types

```rust
#[derive(Debug)]
pub struct TimeoutExpired(pub ());  // maps to Cancelled

#[derive(Debug)]
pub struct ConnectError(pub Box<dyn Error + Send + Sync>);  // maps to Unavailable
```

These are recognized by `find_status_in_source_chain` when walking error sources.

## `into_http` — Status as HTTP Response

```rust
pub fn into_http<B: Default>(self) -> http::Response<B>
```
Builds an HTTP response with:
- `content-type: application/grpc`
- Status headers from `add_header`
- Status inserted into `extensions()` (for downstream access)
- Default body (empty)

This is the "trailers-only" response pattern for immediate errors.

## GrpcMethod Extension

```rust
// extensions.rs
pub struct GrpcMethod<'a> { service: &'a str, method: &'a str }
```
Inserted into `http::Extensions` by generated code so interceptors can
inspect which RPC is being called.

## Notes for Our Implementation

1. **Must adopt boxing pattern** — `Status(Box<StatusInner>)` keeps Result sizes small
2. **Percent-encoding is required** by gRPC spec for `grpc-message`
3. **Base64 for details** — standard alphabet, no padding on encode, indifferent on decode
4. **Source chain walk** is important for composability with tower middleware errors
5. **h2 error mapping** follows the [gRPC HTTP/2 spec](https://github.com/grpc/grpc/blob/3977c30/doc/PROTOCOL-HTTP2.md#errors)
6. **HTTP status mapping** follows the [gRPC HTTP status mapping](https://github.com/grpc/grpc/blob/master/doc/http-grpc-status-mapping.md)
7. We can skip `TimeoutExpired` and `ConnectError` initially — add when transport layer lands
