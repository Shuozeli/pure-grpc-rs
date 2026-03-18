# tonic-build Codegen

**Source:** `tonic-build/src/` (1,973 lines across 5 files)
**Dependencies:** `prettyplease`, `proc-macro2`, `quote`, `syn`

## Architecture

tonic-build is codec-agnostic. It defines generic `Service`/`Method` traits
and generates server traits + client stubs from any implementation.

```
.proto files → prost-build → tonic-prost-build::ServiceGenerator → CodeGenBuilder
                                                                  ├→ generate_server()
                                                                  └→ generate_client()
```

## Core Traits (lib.rs)

```rust
pub trait Service {
    type Comment: AsRef<str>;
    type Method: Method;

    fn name(&self) -> &str;           // "Greeter"
    fn package(&self) -> &str;        // "helloworld"
    fn identifier(&self) -> &str;     // proto name of service
    fn methods(&self) -> &[Self::Method];
    fn comment(&self) -> &[Self::Comment];
}

pub trait Method {
    type Comment: AsRef<str>;

    fn name(&self) -> &str;           // "say_hello" (snake_case)
    fn identifier(&self) -> &str;     // "SayHello" (proto name)
    fn codec_path(&self) -> &str;     // "tonic_prost::ProstCodec"
    fn client_streaming(&self) -> bool;
    fn server_streaming(&self) -> bool;
    fn request_response_name(&self, proto_path, compile_well_known)
        -> (TokenStream, TokenStream);  // Rust type paths
    fn comment(&self) -> &[Self::Comment];
    fn deprecated(&self) -> bool;
}
```

## CodeGenBuilder (code_gen.rs)

```rust
pub struct CodeGenBuilder {
    emit_package: bool,
    compile_well_known_types: bool,
    attributes: Attributes,
    build_transport: bool,
    use_arc_self: bool,
    generate_default_stubs: bool,
    disable_comments: HashSet<String>,
}
```

Central orchestrator. Calls `generate_client()` / `generate_server()`.

## Generated Server Code (server.rs)

For a `Greeter` service with `SayHello` unary RPC:

### 1. Service Trait

```rust
#[async_trait]
pub trait Greeter: Send + Sync + 'static {
    async fn say_hello(
        &self,
        request: tonic::Request<HelloRequest>,
    ) -> Result<tonic::Response<HelloReply>, tonic::Status>;
}
```

For server streaming methods, adds associated type:
```rust
type SayHelloStream: Stream<Item = Result<HelloReply, Status>> + Send + 'static;
```

### 2. Server Wrapper

```rust
pub struct GreeterServer<T> {
    inner: Arc<T>,
    accept_compression_encodings: EnabledCompressionEncodings,
    send_compression_encodings: EnabledCompressionEncodings,
    max_decoding_message_size: Option<usize>,
    max_encoding_message_size: Option<usize>,
}
```

Implements `Service<http::Request<B>>` with path-based dispatch:

```rust
fn call(&mut self, req: http::Request<B>) -> Self::Future {
    match req.uri().path() {
        "/helloworld.Greeter/SayHello" => {
            // Create per-method service struct
            let method = SayHelloSvc(inner.clone());
            let codec = ProstCodec::default();
            let mut grpc = tonic::server::Grpc::new(codec)
                .apply_compression_config(...)
                .apply_max_message_size_config(...);
            let res = grpc.unary(method, req).await;
            Ok(res)
        }
        _ => Status::unimplemented("").into_http()
    }
}
```

### 3. Per-Method Service Structs

```rust
struct SayHelloSvc<T: Greeter>(pub Arc<T>);

impl<T: Greeter> UnaryService<HelloRequest> for SayHelloSvc<T> {
    type Response = HelloReply;
    type Future = BoxFuture<Response<Self::Response>, Status>;
    fn call(&mut self, request: Request<HelloRequest>) -> Self::Future {
        let inner = Arc::clone(&self.0);
        Box::pin(async move { inner.say_hello(request).await })
    }
}
```

### 4. NamedService

```rust
impl<T: Greeter> NamedService for GreeterServer<T> {
    const NAME: &'static str = "helloworld.Greeter";
}
```

## Generated Client Code (client.rs)

```rust
pub struct GreeterClient<T> {
    inner: tonic::client::Grpc<T>,
}
```

### Method Stubs

```rust
// Unary
pub async fn say_hello(&mut self, request: impl IntoRequest<HelloRequest>)
    -> Result<Response<HelloReply>, Status>
{
    self.inner.ready().await?;
    let codec = ProstCodec::default();
    let path = PathAndQuery::from_static("/helloworld.Greeter/SayHello");
    let mut req = request.into_request();
    req.extensions_mut().insert(GrpcMethod::new("helloworld.Greeter", "SayHello"));
    self.inner.unary(req, path, codec).await
}

// Server streaming → self.inner.server_streaming(...)
// Client streaming → self.inner.client_streaming(...)
// Bidi streaming   → self.inner.streaming(...)
```

### Transport Helper (feature-gated)

```rust
impl GreeterClient<Channel> {
    pub async fn connect<D>(dst: D) -> Result<Self, Error>
    where D: TryInto<Endpoint> { ... }
}
```

## Manual Service Builder (manual.rs)

For services without .proto files:

```rust
let service = tonic_build::manual::Service::builder()
    .name("Greeter")
    .package("helloworld")
    .method(
        Method::builder()
            .name("say_hello")
            .route_name("SayHello")
            .input_type("crate::HelloRequest")
            .output_type("crate::HelloReply")
            .codec_path("crate::MyCodec")
            .build(),
    )
    .build();

tonic_build::manual::Builder::new().compile(&[service]);
```

## Notes for Our Implementation

1. **Our codegen will use our protobuf-rs parser** — not prost-build. We need
   our own adapter that implements the Service/Method traits.
2. **The Service/Method trait abstraction is good** — adopt it for codec-agnostic codegen.
3. **Server dispatch is URI-path based** — `"/package.Service/Method"` pattern.
4. **Per-method service structs** are needed to bridge the trait to UnaryService/etc.
5. **Client is simpler** — just wraps `Grpc<T>` and calls the right dispatch method.
6. **For Phase 1, hand-write** the greeter server/client. Move to codegen in Phase 3.
7. **`codec_path`** in Method trait enables pluggable codecs — each method can
   use a different codec if needed.
