# pure-grpc-rs

Pure Rust gRPC framework with pluggable codecs. Built on hyper/h2/tower,
no tonic dependency. Tonic is in `vendor/tonic/` as reference only.

## Quick Orientation

Workspace crates:
- `grpc-core` — Status, Metadata, Codec traits, framing, Body
- `grpc-server` — Server handler, router, hyper serve loop
- `grpc-client` — Client dispatcher, Channel, Endpoint
- `grpc-codegen` — Code generation from service definitions (future)
- `grpc-build` — build.rs entry point (future)
- `grpc-codec-flatbuffers` — FlatBuffers codec (future)

## Git Rules

- Do NOT commit or push unless the user explicitly asks you to
- Do NOT amend commits unless the user explicitly asks you to
- Do NOT force push unless the user explicitly asks you to

## Key Rules

- `vendor/tonic/` is reference only — do NOT add it to the workspace or depend on it
- Do NOT add tonic as a dependency — the whole point is to replace it
- Do NOT use `#[allow(...)]` to bypass clippy or rustdoc warnings — fix the root cause
- Codec system must remain pluggable — never hardcode prost/flatbuffers in core types
- ProstCodec is behind `feature = "prost-codec"`, not a hard dependency
- All 4 RPC patterns must be supported: unary, server-streaming, client-streaming, bidi

## Code Quality Discipline

Shortcuts during exploration are fine — getting things working is the
priority. But tech debt must be visible, not silent.

- **Always leave `// TODO(refactor):` comments** when you take a shortcut.
  `.clone()` that should be `.take()`, duplicated patterns, swallowed
  errors, O(n^2) loops — any of these are OK to ship, but leave the TODO
  so we can track the debt. Silent shortcuts are invisible and accumulate
  until someone does a full code review to discover them.
- **Self-review pass:** After getting the feature working and tests passing,
  re-read the diff once. You don't have to fix everything — just leave
  `// TODO(refactor):` markers on anything you'd flag in a code review.

## Build & Test

```bash
cargo build --workspace                  # must compile
cargo test --workspace                   # all tests
cargo clippy --workspace -- -D warnings  # no warnings
cargo doc --workspace --no-deps          # docs must pass
```

## CI

After pushing, check GitHub Actions status:
```bash
gh run list --limit 3
gh run view <run-id>        # if a run fails
```
