/// Intermediate representation for a gRPC service definition.
///
/// Codec-agnostic — can be built from protobuf or flatbuffers schemas.
#[derive(Debug, Clone)]
pub struct ServiceDef {
    /// The service name (e.g., `"Greeter"`).
    pub name: String,
    /// The fully-qualified package (e.g., `"helloworld"`).
    pub package: String,
    /// The proto identifier (e.g., `"Greeter"`).
    pub proto_name: String,
    /// RPC methods in this service.
    pub methods: Vec<MethodDef>,
    /// Doc comments.
    pub comments: Vec<String>,
}

/// Intermediate representation for a single RPC method.
#[derive(Debug, Clone)]
pub struct MethodDef {
    /// Rust-style method name in snake_case (e.g., `"say_hello"`).
    pub name: String,
    /// Proto-style method name (e.g., `"SayHello"`).
    pub proto_name: String,
    /// Fully-qualified input type path (e.g., `"super::HelloRequest"`).
    pub input_type: String,
    /// Fully-qualified output type path (e.g., `"super::HelloReply"`).
    pub output_type: String,
    /// Whether the client sends a stream of messages.
    pub client_streaming: bool,
    /// Whether the server returns a stream of messages.
    pub server_streaming: bool,
    /// The codec path to use (e.g., `"grpc_core::codec::prost_codec::ProstCodec"`).
    pub codec_path: String,
    /// Doc comments.
    pub comments: Vec<String>,
}

impl ServiceDef {
    /// Fully-qualified gRPC service name (e.g., `"helloworld.Greeter"`).
    pub fn fully_qualified_name(&self) -> String {
        if self.package.is_empty() {
            self.proto_name.clone()
        } else {
            format!("{}.{}", self.package, self.proto_name)
        }
    }
}

impl MethodDef {
    /// gRPC path for this method (e.g., `"/helloworld.Greeter/SayHello"`).
    pub fn grpc_path(&self, service_fqn: &str) -> String {
        debug_assert!(
            !self.proto_name.contains('/'),
            "proto_name must not contain '/', got: {:?}",
            self.proto_name
        );
        format!("/{service_fqn}/{}", self.proto_name)
    }
}

/// Convert comment strings into `#[doc = "..."]` token streams.
pub fn comments_to_doc_tokens(comments: &[String]) -> proc_macro2::TokenStream {
    let attrs: Vec<proc_macro2::TokenStream> = comments
        .iter()
        .map(|c| {
            let line = format!(" {c}");
            quote::quote! { #[doc = #line] }
        })
        .collect();
    quote::quote! { #(#attrs)* }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_service() -> ServiceDef {
        ServiceDef {
            name: "Greeter".into(),
            package: "helloworld".into(),
            proto_name: "Greeter".into(),
            methods: vec![MethodDef {
                name: "say_hello".into(),
                proto_name: "SayHello".into(),
                input_type: "super::HelloRequest".into(),
                output_type: "super::HelloReply".into(),
                client_streaming: false,
                server_streaming: false,
                codec_path: "grpc_core::codec::prost_codec::ProstCodec".into(),
                comments: vec![],
            }],
            comments: vec![],
        }
    }

    #[test]
    fn fully_qualified_name() {
        let svc = sample_service();
        assert_eq!(svc.fully_qualified_name(), "helloworld.Greeter");
    }

    #[test]
    fn fully_qualified_name_no_package() {
        let mut svc = sample_service();
        svc.package = String::new();
        assert_eq!(svc.fully_qualified_name(), "Greeter");
    }

    #[test]
    fn grpc_path() {
        let svc = sample_service();
        let method = &svc.methods[0];
        assert_eq!(
            method.grpc_path(&svc.fully_qualified_name()),
            "/helloworld.Greeter/SayHello"
        );
    }
}
