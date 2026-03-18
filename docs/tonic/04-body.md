# Body

**Source:** `tonic/src/body.rs` (94 lines)
**Dependencies:** `http-body`, `http-body-util`, `bytes`

## Overview

Type-erased HTTP body. Wraps `http_body_util::combinators::UnsyncBoxBody`
with an optimization for empty bodies.

## Type Definitions

```rust
type BoxBody = http_body_util::combinators::UnsyncBoxBody<bytes::Bytes, crate::Status>;

#[derive(Debug)]
pub struct Body {
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    Empty,
    Wrap(BoxBody),
}
```

## Key Design: `Body::new` Constructor

```rust
pub fn new<B>(body: B) -> Self
where
    B: http_body::Body<Data = bytes::Bytes> + Send + 'static,
    B::Error: Into<crate::BoxError>,
```

Three-step optimization:
1. **Empty fast path**: if `body.is_end_stream()`, return `Self::empty()`
2. **Downcast avoidance**: if `B` is already `Body`, unwrap and return directly
3. **Downcast avoidance**: if `B` is already `BoxBody`, wrap directly
4. **Fallback**: map errors through `Status::map_error`, then `boxed_unsync()`

The downcast trick uses `<dyn Any>::downcast_mut::<Option<T>>` on a local
`Option<B>` to avoid double-boxing.

## Body Trait Implementation

```rust
impl http_body::Body for Body {
    type Data = bytes::Bytes;
    type Error = crate::Status;

    fn poll_frame(...)     // delegates to Kind
    fn size_hint(...)      // Empty => exact(0), Wrap => delegate
    fn is_end_stream(...)  // Empty => true, Wrap => delegate
}
```

## Notes for Our Implementation

1. **Keep it simple** — this is only 94 lines but critical as the type-erased body
2. **UnsyncBoxBody** — `Unsync` means no `Sync` bound, fine for HTTP/2 streams
   (each stream is pinned to one task)
3. **Error type is `Status`** — all body errors are mapped to gRPC Status
4. **Default is empty** — `impl Default for Body` returns `Body::empty()`
5. We may want to use a type alias instead of a wrapper initially:
   `pub type Body = UnsyncBoxBody<Bytes, Status>;` — simpler, optimize later
