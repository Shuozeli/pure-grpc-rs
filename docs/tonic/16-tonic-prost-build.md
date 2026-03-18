# tonic-prost-build

**Source:** `tonic-prost-build/src/lib.rs` (963 lines + 318 lines tests)
**Dependencies:** `tonic-build`, `prost-build`, `prost-types`, `prettyplease`, `syn`

## Overview

Bridges prost-build (protobuf compiler) and tonic-build (gRPC code generator).
Implements prost-build's `ServiceGenerator` trait to intercept service definitions
and generate gRPC client/server code.

## Builder

```rust
pub struct Builder {
    build_client: bool,
    build_server: bool,
    build_transport: bool,
    out_dir: Option<PathBuf>,
    extern_path: Vec<(String, String)>,
    field_attributes: Vec<(String, String)>,
    type_attributes: Vec<(String, String)>,
    server_attributes: Attributes,
    client_attributes: Attributes,
    proto_path: String,                    // default: "super"
    compile_well_known_types: bool,
    codec_path: String,                    // default: "tonic_prost::ProstCodec"
    use_arc_self: bool,
    generate_default_stubs: bool,
    emit_rerun_if_changed: bool,
    // ... many more
}
```

### Usage

```rust
// Simple
tonic_prost_build::compile_protos(&["proto/hello.proto"], &["proto"])?;

// Configured
tonic_prost_build::configure()
    .build_client(true)
    .build_server(true)
    .codec_path("my_codec::MyCodec")
    .compile_protos(&["proto/hello.proto"], &["proto"])?;
```

## ServiceGenerator

```rust
impl prost_build::ServiceGenerator for ServiceGenerator {
    fn generate(&mut self, service: prost_build::Service, buf: &mut String) {
        // 1. Wrap prost Service/Method in tonic trait adapters
        let tonic_service = TonicBuildService::new(service, self.codec_path.clone());

        // 2. Create CodeGenBuilder with configuration
        let mut builder = CodeGenBuilder::new();
        builder.emit_package(true)
               .build_transport(self.build_transport)
               .use_arc_self(self.use_arc_self)
               .generate_default_stubs(self.generate_default_stubs);

        // 3. Generate client and/or server code
        let mut tokens = TokenStream::new();
        if self.build_client {
            tokens.extend(builder.generate_client(&tonic_service, &self.proto_path));
        }
        if self.build_server {
            tokens.extend(builder.generate_server(&tonic_service, &self.proto_path));
        }

        // 4. Format with prettyplease and write to buffer
        let formatted = prettyplease::unparse(&syn::parse2(tokens).unwrap());
        buf.push_str(&formatted);
    }
}
```

## Prost-to-Tonic Adapters

```rust
struct TonicBuildService {
    prost_service: prost_build::Service,
    methods: Vec<TonicBuildMethod>,
}

struct TonicBuildMethod {
    prost_method: prost_build::Method,
    codec_path: String,
}
```

These implement tonic-build's `Service`/`Method` traits, delegating to prost types.

### Well-Known Types

Special mapping when `compile_well_known_types = false`:

| Proto Type | Rust Type |
|-----------|-----------|
| `.google.protobuf.Empty` | `()` |
| `.google.protobuf.Any` | `::prost_types::Any` |
| `.google.protobuf.StringValue` | `::prost::alloc::string::String` |
| `.google.protobuf.BoolValue` | `bool` |
| etc. | etc. |

## compile_protos Flow

```
1. Create prost_build::Config
2. Register ServiceGenerator with config.service_generator(gen)
3. config.compile_protos(protos, includes)?
   └── For each .proto:
       ├── Generate message types (prost)
       └── For each service:
           └── call ServiceGenerator::generate()
               ├── Wrap in TonicBuildService
               └── Generate client/server via CodeGenBuilder
4. Output written to OUT_DIR/{package_name}.rs
```

## Notes for Our Implementation

1. **We replace this entire crate** — our protobuf-rs compiler handles parsing
2. **Our equivalent**: `grpc-codegen` + `grpc-build`
   - `grpc-codegen` has the IR (ServiceDef, MethodDef) + generation logic
   - `grpc-build` is the build.rs entry point
3. **ServiceDef/MethodDef** is our equivalent of TonicBuildService/TonicBuildMethod
4. **We need an adapter from our protobuf-rs AST** to ServiceDef
5. **codec_path** is configurable — for protobuf-rs we'd use our own codec path
6. **prettyplease** is a good formatter choice for generated code
