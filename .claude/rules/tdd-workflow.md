# Test-Driven Development Workflow

## Process

For every module, follow this strict order:

1. **Define the trait / public API** — Write the type signatures, trait definitions, and public struct declarations with method stubs. This is the contract.
2. **Write unit tests** — Test the public behavior against the trait/API. Tests should compile but fail (or not compile if stubs are unimplemented).
3. **Write the implementation** — Fill in the logic to make the tests pass.

## Rules

- Never write implementation before tests exist for that behavior.
- Tests test public behavior only. Do not test private internals.
- Each test should test one thing and have a clear name describing what it verifies.
- Use `todo!()` or `unimplemented!()` for method stubs during step 1.
- After implementation, run `cargo test` and `cargo clippy` before moving on.

## Example Flow

```
1. Define:   pub trait Encoder { fn encode(&mut self, item, buf) -> Result<...>; }
2. Test:     #[test] fn encode_writes_bytes_to_buffer() { ... }
3. Implement: fill in encode() logic
4. Verify:   cargo test && cargo clippy
```
