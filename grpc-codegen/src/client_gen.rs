use crate::ir::{comments_to_doc_tokens, MethodDef, ServiceDef, StreamingType};
use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Generate client code for a service definition.
///
/// Produces an `XxxClient<T>` struct with one async method per RPC.
pub fn generate(service: &ServiceDef) -> TokenStream {
    let errors = service.validate();
    assert!(
        errors.is_empty(),
        "invalid ServiceDef for client codegen: {}",
        errors.join("; ")
    );
    let client_name = format_ident!("{}Client", service.name);
    let mod_name = format_ident!("{}_client", service.name.to_snake_case());
    let fqn = service.fully_qualified_name();

    let service_docs = comments_to_doc_tokens(&service.comments);
    let connect_method = gen_connect_method(&client_name);
    let rpc_methods = service.methods.iter().map(|m| gen_rpc_method(m, &fqn));

    // Streaming-only imports: only emit when at least one method actually
    // uses streaming. Same rationale as `server_gen` -- unary-only
    // contracts should not transitively pull in streaming traits.
    let has_server_or_bidi = service
        .methods
        .iter()
        .any(|m| matches!(m.streaming, StreamingType::Server | StreamingType::BiDi));
    let has_client_or_bidi = service
        .methods
        .iter()
        .any(|m| matches!(m.streaming, StreamingType::Client | StreamingType::BiDi));
    let streaming_response_import = if has_server_or_bidi {
        quote! { use grpc_core::codec::Streaming; }
    } else {
        quote! {}
    };
    let streaming_request_import = if has_client_or_bidi {
        quote! { use grpc_core::request::IntoStreamingRequest; }
    } else {
        quote! {}
    };

    quote! {
        #[allow(unused_imports)]
        pub mod #mod_name {
            use grpc_client::{Grpc, GrpcService};
            use grpc_core::body::Body;
            #streaming_response_import
            use grpc_core::extensions::GrpcMethod;
            use grpc_core::request::IntoRequest;
            #streaming_request_import
            use grpc_core::{Response, Status};
            use http::uri::PathAndQuery;
            use http_body::Body as HttpBody;

            #service_docs
            #[derive(Debug, Clone)]
            pub struct #client_name<T> {
                inner: Grpc<T>,
            }

            #connect_method

            impl<T> #client_name<T> {
                pub fn new(inner: T) -> Self {
                    let inner = Grpc::new(inner);
                    Self { inner }
                }

                pub fn with_origin(inner: T, origin: http::Uri) -> Self {
                    let inner = Grpc::with_origin(inner, origin);
                    Self { inner }
                }
            }

            impl<T> #client_name<T>
            where
                T: GrpcService<Body>,
                T::Error: Into<grpc_core::BoxError>,
                T::ResponseBody: HttpBody<Data = bytes::Bytes> + Send + 'static,
                <T::ResponseBody as HttpBody>::Error: Into<grpc_core::BoxError>,
            {
                #(#rpc_methods)*
            }
        }
    }
}

fn gen_connect_method(client_name: &proc_macro2::Ident) -> TokenStream {
    quote! {
        impl #client_name<grpc_client::Channel> {
            pub async fn connect(
                uri: http::Uri,
            ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                let channel = grpc_client::Channel::connect(uri.clone()).await?;
                Ok(Self {
                    inner: Grpc::with_origin(channel, uri),
                })
            }
        }
    }
}

fn gen_rpc_method(method: &MethodDef, service_fqn: &str) -> TokenStream {
    let method_docs = comments_to_doc_tokens(&method.comments);
    let name = format_ident!("{}", method.rust_name());
    let path = method.grpc_path(service_fqn);
    let proto_name = method.name.as_str();
    let codec_path: TokenStream = method
        .codec_path
        .parse()
        .unwrap_or_else(|e| panic!("invalid codec_path `{}`: {e}", method.codec_path));
    let input: TokenStream = method
        .input_type
        .parse()
        .unwrap_or_else(|e| panic!("invalid input_type `{}`: {e}", method.input_type));
    let output: TokenStream = method
        .output_type
        .parse()
        .unwrap_or_else(|e| panic!("invalid output_type `{}`: {e}", method.output_type));
    let service_fqn_str = service_fqn;

    let ready_check = quote! {
        self.inner.ready().await
            .map_err(|e| Status::unknown(
                format!("Service was not ready: {}", e.into())
            ))?;
    };

    match method.streaming {
        StreamingType::None => {
            // Unary
            quote! {
                #method_docs
                pub async fn #name(
                    &mut self,
                    request: impl IntoRequest<#input>,
                ) -> Result<Response<#output>, Status> {
                    #ready_check
                    let codec = #codec_path::<#input, #output>::default();
                    let path = PathAndQuery::from_static(#path);
                    let mut req = request.into_request();
                    req.extensions_mut()
                        .insert(GrpcMethod::new(#service_fqn_str, #proto_name));
                    self.inner.unary(req, path, codec).await
                }
            }
        }
        StreamingType::Server => {
            // Server streaming
            quote! {
                #method_docs
                pub async fn #name(
                    &mut self,
                    request: impl IntoRequest<#input>,
                ) -> Result<Response<Streaming<#output>>, Status> {
                    #ready_check
                    let codec = #codec_path::<#input, #output>::default();
                    let path = PathAndQuery::from_static(#path);
                    let mut req = request.into_request();
                    req.extensions_mut()
                        .insert(GrpcMethod::new(#service_fqn_str, #proto_name));
                    self.inner.server_streaming(req, path, codec).await
                }
            }
        }
        StreamingType::Client => {
            // Client streaming
            quote! {
                #method_docs
                pub async fn #name(
                    &mut self,
                    request: impl IntoStreamingRequest<Message = #input>,
                ) -> Result<Response<#output>, Status> {
                    #ready_check
                    let codec = #codec_path::<#input, #output>::default();
                    let path = PathAndQuery::from_static(#path);
                    let mut req = request.into_streaming_request();
                    req.extensions_mut()
                        .insert(GrpcMethod::new(#service_fqn_str, #proto_name));
                    self.inner.client_streaming(req, path, codec).await
                }
            }
        }
        StreamingType::BiDi => {
            // Bidi streaming
            quote! {
                #method_docs
                pub async fn #name(
                    &mut self,
                    request: impl IntoStreamingRequest<Message = #input>,
                ) -> Result<Response<Streaming<#output>>, Status> {
                    #ready_check
                    let codec = #codec_path::<#input, #output>::default();
                    let path = PathAndQuery::from_static(#path);
                    let mut req = request.into_streaming_request();
                    req.extensions_mut()
                        .insert(GrpcMethod::new(#service_fqn_str, #proto_name));
                    self.inner.streaming(req, path, codec).await
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{MethodDef, ServiceDef, StreamingType};

    fn sample_service() -> ServiceDef {
        ServiceDef {
            name: "Greeter".into(),
            package: Some("helloworld".into()),
            comments: vec![],
            methods: vec![MethodDef {
                name: "SayHello".into(),
                rust_name: Some("say_hello".into()),
                input_type: "super::HelloRequest".into(),
                output_type: "super::HelloReply".into(),
                streaming: StreamingType::None,
                codec_path: "grpc_core::codec::prost_codec::ProstCodec".into(),
                comments: vec![],
            }],
        }
    }

    #[test]
    fn generates_parseable_token_stream() {
        let svc = sample_service();
        let tokens = generate(&svc);
        let file = syn::parse2::<syn::File>(tokens);
        assert!(
            file.is_ok(),
            "generated code should parse: {:?}",
            file.err()
        );
    }

    #[test]
    fn generated_code_contains_client() {
        let svc = sample_service();
        let tokens = generate(&svc);
        let code = tokens.to_string();
        assert!(
            code.contains("GreeterClient"),
            "should contain client struct"
        );
        assert!(code.contains("say_hello"), "should contain method");
        assert!(code.contains("connect"), "should contain connect method");
    }

    use crate::test_util::module_items;

    #[test]
    fn generated_client_has_expected_methods() {
        let svc = sample_service();
        let tokens = generate(&svc);
        let file: syn::File = syn::parse2(tokens).unwrap();
        let items = module_items(&file);

        // Collect all method names from impl blocks
        let method_names: Vec<String> = items
            .iter()
            .filter_map(|item| {
                if let syn::Item::Impl(imp) = item {
                    Some(imp)
                } else {
                    None
                }
            })
            .flat_map(|imp| &imp.items)
            .filter_map(|item| {
                if let syn::ImplItem::Fn(m) = item {
                    Some(m.sig.ident.to_string())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            method_names.contains(&"say_hello".to_string()),
            "client should have say_hello method, got: {method_names:?}"
        );
        assert!(
            method_names.contains(&"connect".to_string()),
            "client should have connect method, got: {method_names:?}"
        );
    }

    #[test]
    fn all_four_rpc_patterns_generate_parseable_code() {
        let svc = ServiceDef {
            name: "TestSvc".into(),
            package: Some("test".into()),
            comments: vec![],
            methods: vec![
                MethodDef {
                    name: "Unary".into(),
                    rust_name: Some("unary".into()),
                    input_type: "super::Req".into(),
                    output_type: "super::Resp".into(),
                    streaming: StreamingType::None,
                    codec_path: "Codec".into(),
                    comments: vec![],
                },
                MethodDef {
                    name: "ServerStream".into(),
                    rust_name: Some("server_stream".into()),
                    input_type: "super::Req".into(),
                    output_type: "super::Resp".into(),
                    streaming: StreamingType::Server,
                    codec_path: "Codec".into(),
                    comments: vec![],
                },
                MethodDef {
                    name: "ClientStream".into(),
                    rust_name: Some("client_stream".into()),
                    input_type: "super::Req".into(),
                    output_type: "super::Resp".into(),
                    streaming: StreamingType::Client,
                    codec_path: "Codec".into(),
                    comments: vec![],
                },
                MethodDef {
                    name: "BiDi".into(),
                    rust_name: Some("bidi".into()),
                    input_type: "super::Req".into(),
                    output_type: "super::Resp".into(),
                    streaming: StreamingType::BiDi,
                    codec_path: "Codec".into(),
                    comments: vec![],
                },
            ],
        };

        let tokens = generate(&svc);
        let file = syn::parse2::<syn::File>(tokens);
        assert!(
            file.is_ok(),
            "all 4 RPC patterns should generate parseable code: {:?}",
            file.err()
        );
    }
}
