# Metadata

**Source:** `tonic/src/metadata/` (4,286 lines across 5 files)
**Dependencies:** `bytes`, `http` (HeaderMap, HeaderName, HeaderValue), `base64`

## Overview

`MetadataMap` wraps `http::HeaderMap` to provide gRPC-specific metadata
handling. The key insight is the **phantom type encoding pattern**: keys and
values are parameterized by `Ascii` or `Binary` marker types, enforcing
at compile time that `-bin` suffixed keys use binary encoding (base64).

## Type Hierarchy

```
MetadataMap
  └── headers: http::HeaderMap       (the actual storage)

MetadataKey<VE: ValueEncoding>
  ├── inner: http::HeaderName        (repr(transparent))
  └── phantom: PhantomData<VE>

MetadataValue<VE: ValueEncoding>
  ├── inner: http::HeaderValue       (repr(transparent))
  └── phantom: PhantomData<VE>

Type aliases:
  AsciiMetadataKey  = MetadataKey<Ascii>
  BinaryMetadataKey = MetadataKey<Binary>
  AsciiMetadataValue  = MetadataValue<Ascii>
  BinaryMetadataValue = MetadataValue<Binary>
```

## Encoding Marker Types

```rust
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum Ascii {}   // zero-sized, uninhabitable

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum Binary {}  // zero-sized, uninhabitable
```

These are used purely as type parameters. `#[non_exhaustive]` + no variants
makes them impossible to instantiate.

## ValueEncoding Trait

```rust
pub trait ValueEncoding: Clone + Eq + PartialEq + Hash + value_encoding::Sealed {
    fn is_valid_key(key: &str) -> bool;
}

// Sealed trait with the actual encoding operations:
trait Sealed {
    fn is_empty(value: &[u8]) -> bool;
    fn from_bytes(value: &[u8]) -> Result<HeaderValue, InvalidMetadataValueBytes>;
    fn from_shared(value: Bytes) -> Result<HeaderValue, InvalidMetadataValueBytes>;
    fn from_static(value: &'static str) -> HeaderValue;
    fn decode(value: &[u8]) -> Result<Bytes, InvalidMetadataValueBytes>;
    fn equals(a: &HeaderValue, b: &[u8]) -> bool;
    fn values_equal(a: &HeaderValue, b: &HeaderValue) -> bool;
    fn fmt(value: &HeaderValue, f: &mut fmt::Formatter) -> fmt::Result;
}
```

### Ascii Implementation

| Method | Behavior |
|--------|----------|
| `is_valid_key` | `!key.ends_with("-bin")` |
| `is_empty` | `value.is_empty()` |
| `from_bytes` | Direct `HeaderValue::from_bytes` (visible ASCII 32-127) |
| `decode` | Returns bytes as-is |
| `equals` | Direct byte comparison |

### Binary Implementation

| Method | Behavior |
|--------|----------|
| `is_valid_key` | `key.ends_with("-bin")` |
| `is_empty` | All bytes are `=` (base64 padding only) |
| `from_bytes` | **Encodes** input to base64 (STANDARD_NO_PAD), stores encoded |
| `from_shared` | Delegates to `from_bytes` (re-encodes) |
| `from_static` | Validates input IS base64, stores as-is |
| `decode` | **Decodes** from base64 (STANDARD with padding) |
| `equals` | Decodes `a` from base64, compares decoded bytes to `b` |
| `values_equal` | Decodes both, compares decoded bytes |

**Key insight:** Binary values are stored base64-encoded in the HeaderValue
(since HTTP headers are text), but the API accepts/returns raw bytes.

## MetadataKey

```rust
#[repr(transparent)]
pub struct MetadataKey<VE: ValueEncoding> {
    pub(crate) inner: http::header::HeaderName,
    phantom: PhantomData<VE>,
}
```

- `repr(transparent)` enables unsafe pointer casts from `&HeaderName` to `&MetadataKey`
- `from_bytes` validates via `HeaderName::from_bytes` AND `VE::is_valid_key`
- `from_static` panics if key doesn't match encoding type
- `unchecked_from_header_name_ref` uses unsafe transmute (same memory layout)

## MetadataValue

```rust
#[repr(transparent)]
pub struct MetadataValue<VE: ValueEncoding> {
    pub(crate) inner: HeaderValue,
    phantom: PhantomData<VE>,
}
```

- `repr(transparent)` enables unsafe pointer casts
- `from_static` — for Ascii: direct; for Binary: validates base64
- `from_bytes` — for Binary only: base64-encodes then stores
- `to_bytes()` — decodes (Ascii: copy, Binary: base64 decode)
- `as_encoded_bytes()` — raw header bytes (base64 for Binary)
- Supports integer conversions (u16, i16, u32, i32, u64, i64)
- `set_sensitive` / `is_sensitive` — HPACK never-index hint

## MetadataMap

```rust
#[derive(Clone, Debug, Default)]
pub struct MetadataMap {
    headers: http::HeaderMap,
}
```

### Reserved Headers

```rust
const GRPC_RESERVED_HEADERS: [HeaderName; 5] = [
    "te", "content-type", "grpc-message", "grpc-message-type", "grpc-status"
];
```

`into_sanitized_headers()` strips these before inserting custom metadata into
status trailing headers.

### Dual API Pattern

Methods come in pairs — ascii and binary:
- `get(key)` / `get_bin(key)`
- `insert(key, val)` / `insert_bin(key, val)`
- `append(key, val)` / `append_bin(key, val)`
- `remove(key)` / `remove_bin(key)`
- `entry(key)` / `entry_bin(key)`
- `get_all(key)` / `get_all_bin(key)`

The type system prevents mixing: `get("foo-bin")` returns `None` because
`Ascii::is_valid_key("foo-bin")` is false.

### Conversions

- `from_headers(HeaderMap)` — wraps existing HeaderMap
- `into_headers()` — unwraps to HeaderMap
- `into_sanitized_headers()` — strips reserved headers, then unwraps
- `AsRef<HeaderMap>` / `AsMut<HeaderMap>` — direct access

### Iterator Types

Rich iterator support mirroring `http::HeaderMap`:
- `Iter` / `IterMut` — yields `KeyAndValueRef` / `KeyAndMutValueRef` enums
- `Keys` — yields `KeyRef` enum (Ascii or Binary)
- `Values` / `ValuesMut` — yields `ValueRef` / `ValueRefMut` enums
- `ValueIter<VE>` / `GetAll<VE>` — values for a single key
- `ValueDrain<VE>` — draining iterator
- `Entry<VE>` — `Occupied` / `Vacant` pattern

## Constants

```rust
pub const GRPC_CONTENT_TYPE: HeaderValue = HeaderValue::from_static("application/grpc");
pub(crate) const GRPC_TIMEOUT_HEADER: &str = "grpc-timeout";
```

## Notes for Our Implementation

1. **Keep the phantom type pattern** — it's zero-cost and prevents encoding bugs
2. **`repr(transparent)` is load-bearing** — enables the unsafe transmutes between
   `HeaderName`/`MetadataKey` and `HeaderValue`/`MetadataValue`. We should adopt
   this for zero-cost conversions.
3. **The map is just a thin wrapper over `http::HeaderMap`** — most methods delegate
   directly. Don't reinvent this.
4. **Binary values are base64 STANDARD_NO_PAD on encode, STANDARD (padded) on decode**
   — must handle both padded and unpadded inputs.
5. **The dual API (`get`/`get_bin`) is verbose but type-safe** — consider whether we
   want to simplify. Tonic's approach is maximally safe.
6. **The 2,745-line map.rs is mostly boilerplate** — iterator impls, trait impls,
   entry API. We can start with a much smaller subset (get/insert/remove/iter).
7. **Sealed trait pattern** prevents external implementations of `ValueEncoding`.
