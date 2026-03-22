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

    /// Validate that this definition will produce correct generated code.
    ///
    /// Returns a list of problems. An empty vec means the definition is valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.name.is_empty() {
            errors.push("service name is empty".into());
        }
        if self.proto_name.is_empty() {
            errors.push("service proto_name is empty".into());
        }
        for (i, m) in self.methods.iter().enumerate() {
            if m.name.is_empty() {
                errors.push(format!("method[{i}] name is empty"));
            }
            if m.proto_name.is_empty() {
                errors.push(format!("method[{i}] proto_name is empty"));
            }
            if m.proto_name.contains('/') {
                errors.push(format!(
                    "method[{i}] proto_name `{}` contains '/'",
                    m.proto_name
                ));
            }
            if m.input_type.is_empty() {
                errors.push(format!("method[{i}] `{}` input_type is empty", m.name));
            }
            if m.output_type.is_empty() {
                errors.push(format!("method[{i}] `{}` output_type is empty", m.name));
            }
        }
        errors
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

    #[test]
    fn validate_valid_service() {
        let svc = sample_service();
        assert!(svc.validate().is_empty());
    }

    #[test]
    fn validate_catches_empty_name() {
        let mut svc = sample_service();
        svc.name = String::new();
        let errors = svc.validate();
        assert!(errors.iter().any(|e| e.contains("service name is empty")));
    }

    #[test]
    fn validate_catches_slash_in_proto_name() {
        let mut svc = sample_service();
        svc.methods[0].proto_name = "Say/Hello".into();
        let errors = svc.validate();
        assert!(errors.iter().any(|e| e.contains("contains '/'")));
    }

    /// G1: Test comments_to_doc_tokens directly.
    #[test]
    fn comments_to_doc_tokens_empty() {
        let tokens = comments_to_doc_tokens(&[]);
        assert!(tokens.is_empty(), "no comments should produce empty tokens");
    }

    /// G1: Test comments_to_doc_tokens with single comment.
    #[test]
    fn comments_to_doc_tokens_single() {
        let tokens = comments_to_doc_tokens(&["Hello world".into()]);
        let s = tokens.to_string();
        assert!(s.contains("doc"), "should produce #[doc] attributes");
        assert!(s.contains("Hello world"), "should contain comment text");
    }

    /// G1: Test comments_to_doc_tokens with multiple comments.
    #[test]
    fn comments_to_doc_tokens_multiple() {
        let comments = vec![
            "First line".into(),
            "Second line".into(),
            "Third line".into(),
        ];
        let tokens = comments_to_doc_tokens(&comments);
        let s = tokens.to_string();

        // Should produce three #[doc] attributes
        let doc_count = s.matches("doc").count();
        assert_eq!(doc_count, 3, "should produce 3 doc attributes, got: {s}");
        assert!(s.contains("First line"));
        assert!(s.contains("Second line"));
        assert!(s.contains("Third line"));
    }
}
