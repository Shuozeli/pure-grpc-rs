//! Intermediate representation for gRPC service definitions.
//!
//! Re-exports types from [`codegen_schema`] and adds gRPC-specific helpers.

pub use codegen_schema::{MethodDef, ServiceDef, StreamingType};

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
            package: Some("helloworld".into()),
            methods: vec![MethodDef {
                name: "SayHello".into(),
                rust_name: None,
                input_type: "HelloRequest".into(),
                output_type: "HelloReply".into(),
                streaming: StreamingType::None,
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
        let svc = ServiceDef {
            name: "Greeter".into(),
            package: None,
            methods: vec![],
            comments: vec![],
        };
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
    fn streaming_bools_none() {
        let method = MethodDef {
            name: "Unary".into(),
            rust_name: None,
            input_type: "Req".into(),
            output_type: "Resp".into(),
            streaming: StreamingType::None,
            codec_path: "Codec".into(),
            comments: vec![],
        };
        let (client, server) = match method.streaming {
            StreamingType::None => (false, false),
            StreamingType::Server => (false, true),
            StreamingType::Client => (true, false),
            StreamingType::BiDi => (true, true),
        };
        assert_eq!((client, server), (false, false));
    }

    #[test]
    fn streaming_bools_server() {
        let method = MethodDef {
            name: "ServerStream".into(),
            rust_name: None,
            input_type: "Req".into(),
            output_type: "Resp".into(),
            streaming: StreamingType::Server,
            codec_path: "Codec".into(),
            comments: vec![],
        };
        let (client, server) = match method.streaming {
            StreamingType::None => (false, false),
            StreamingType::Server => (false, true),
            StreamingType::Client => (true, false),
            StreamingType::BiDi => (true, true),
        };
        assert_eq!((client, server), (false, true));
    }

    #[test]
    fn streaming_bools_client() {
        let method = MethodDef {
            name: "ClientStream".into(),
            rust_name: None,
            input_type: "Req".into(),
            output_type: "Resp".into(),
            streaming: StreamingType::Client,
            codec_path: "Codec".into(),
            comments: vec![],
        };
        let (client, server) = match method.streaming {
            StreamingType::None => (false, false),
            StreamingType::Server => (false, true),
            StreamingType::Client => (true, false),
            StreamingType::BiDi => (true, true),
        };
        assert_eq!((client, server), (true, false));
    }

    #[test]
    fn streaming_bools_bidi() {
        let method = MethodDef {
            name: "BiDi".into(),
            rust_name: None,
            input_type: "Req".into(),
            output_type: "Resp".into(),
            streaming: StreamingType::BiDi,
            codec_path: "Codec".into(),
            comments: vec![],
        };
        let (client, server) = match method.streaming {
            StreamingType::None => (false, false),
            StreamingType::Server => (false, true),
            StreamingType::Client => (true, false),
            StreamingType::BiDi => (true, true),
        };
        assert_eq!((client, server), (true, true));
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
