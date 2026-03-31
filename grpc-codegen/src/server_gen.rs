use crate::ir::{comments_to_doc_tokens, MethodDef, ServiceDef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Build the response-stream associated type name for a streaming method.
///
/// Strips a trailing "Stream" from `proto_name` before appending "ResponseStream"
/// to avoid stutter like `SayHelloStreamResponseStream`.
fn response_stream_ident(proto_name: &str) -> proc_macro2::Ident {
    let base = proto_name.strip_suffix("Stream").unwrap_or(proto_name);
    format_ident!("{}ResponseStream", base)
}

/// Generate server code for a service definition.
///
/// Produces:
/// - A trait with one method per RPC
/// - An `XxxServer<T>` wrapper struct implementing `tower::Service`
/// - Per-method service structs bridging the trait to UnaryService/etc.
pub fn generate(service: &ServiceDef) -> TokenStream {
    let errors = service.validate();
    assert!(
        errors.is_empty(),
        "invalid ServiceDef for server codegen: {}",
        errors.join("; ")
    );
    let service_name = format_ident!("{}", service.name);
    let server_name = format_ident!("{}Server", service.name);
    let mod_name = format_ident!("{}_server", service.name.to_snake_case());
    let fqn = service.fully_qualified_name();
    let service_docs = comments_to_doc_tokens(&service.comments);

    let trait_methods = service.methods.iter().map(gen_trait_method);
    let trait_assoc_types = service
        .methods
        .iter()
        .filter(|m| m.server_streaming)
        .map(gen_trait_assoc_type);

    let svc_structs = service
        .methods
        .iter()
        .map(|m| gen_svc_struct(m, &service_name));
    let dispatch_arms = service.methods.iter().map(|m| gen_dispatch_arm(m, &fqn));

    quote! {
        pub mod #mod_name {
            use std::convert::Infallible;
            use std::future::Future;
            use std::pin::Pin;
            use std::sync::Arc;
            use std::task::{Context, Poll};
            use tokio_stream::Stream;
            use grpc_core::body::Body;
            use grpc_core::{Request, Response, Status, Streaming};

            type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

            #service_docs
            pub trait #service_name: Send + Sync + 'static {
                #(#trait_assoc_types)*
                #(#trait_methods)*
            }

            pub struct #server_name<T> {
                inner: Arc<T>,
            }

            impl<T> Clone for #server_name<T> {
                fn clone(&self) -> Self {
                    Self { inner: Arc::clone(&self.inner) }
                }
            }

            impl<T: #service_name> #server_name<T> {
                pub fn new(inner: T) -> Self {
                    Self { inner: Arc::new(inner) }
                }
            }

            impl<T: #service_name> grpc_server::NamedService for #server_name<T> {
                const NAME: &'static str = #fqn;
            }

            #(#svc_structs)*

            impl<T, B> tower_service::Service<http::Request<B>> for #server_name<T>
            where
                T: #service_name,
                B: grpc_core::http_body::Body<Data = grpc_core::bytes::Bytes> + Send + 'static,
                B::Error: Into<grpc_core::BoxError>,
            {
                type Response = http::Response<Body>;
                type Error = Infallible;
                type Future = BoxFuture<Result<http::Response<Body>, Infallible>>;

                fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
                    Poll::Ready(Ok(()))
                }

                fn call(&mut self, req: http::Request<B>) -> Self::Future {
                    let (parts, body) = req.into_parts();
                    let req = http::Request::from_parts(parts, Body::new(body));
                    let inner = self.inner.clone();
                    match req.uri().path() {
                        #(#dispatch_arms)*
                        _ => Box::pin(async {
                            let status = Status::unimplemented("method not found");
                            Ok(status.into_http())
                        }),
                    }
                }
            }
        }
    }
}

fn gen_trait_method(method: &MethodDef) -> TokenStream {
    let name = format_ident!("{}", method.name);
    let input: TokenStream = method
        .input_type
        .parse()
        .unwrap_or_else(|e| panic!("invalid input_type `{}`: {e}", method.input_type));
    let output: TokenStream = method
        .output_type
        .parse()
        .unwrap_or_else(|e| panic!("invalid output_type `{}`: {e}", method.output_type));

    match (method.client_streaming, method.server_streaming) {
        (false, false) => {
            // Unary
            quote! {
                fn #name(&self, request: Request<#input>)
                    -> BoxFuture<Result<Response<#output>, Status>>;
            }
        }
        (false, true) => {
            // Server streaming
            let stream_type = response_stream_ident(&method.proto_name);
            quote! {
                fn #name(&self, request: Request<#input>)
                    -> BoxFuture<Result<Response<Self::#stream_type>, Status>>;
            }
        }
        (true, false) => {
            // Client streaming
            quote! {
                fn #name(&self, request: Request<Streaming<#input>>)
                    -> BoxFuture<Result<Response<#output>, Status>>;
            }
        }
        (true, true) => {
            // Bidi streaming
            let stream_type = response_stream_ident(&method.proto_name);
            quote! {
                fn #name(&self, request: Request<Streaming<#input>>)
                    -> BoxFuture<Result<Response<Self::#stream_type>, Status>>;
            }
        }
    }
}

fn gen_trait_assoc_type(method: &MethodDef) -> TokenStream {
    let stream_type = response_stream_ident(&method.proto_name);
    let output: TokenStream = method
        .output_type
        .parse()
        .unwrap_or_else(|e| panic!("invalid output_type `{}`: {e}", method.output_type));
    quote! {
        type #stream_type: Stream<Item = Result<#output, Status>> + Send + 'static;
    }
}

fn gen_svc_struct(method: &MethodDef, service_name: &proc_macro2::Ident) -> TokenStream {
    let svc_name = format_ident!("{}Svc", method.name.to_upper_camel_case());
    let method_name = format_ident!("{}", method.name);
    let input: TokenStream = method
        .input_type
        .parse()
        .unwrap_or_else(|e| panic!("invalid input_type `{}`: {e}", method.input_type));
    let output: TokenStream = method
        .output_type
        .parse()
        .unwrap_or_else(|e| panic!("invalid output_type `{}`: {e}", method.output_type));

    match (method.client_streaming, method.server_streaming) {
        (false, false) => {
            quote! {
                struct #svc_name<T: #service_name>(Arc<T>);
                impl<T: #service_name> grpc_server::UnaryService<#input> for #svc_name<T> {
                    type Response = #output;
                    type Future = BoxFuture<Result<Response<#output>, Status>>;
                    fn call(&mut self, request: Request<#input>) -> Self::Future {
                        let inner = Arc::clone(&self.0);
                        Box::pin(async move { inner.#method_name(request).await })
                    }
                }
            }
        }
        (false, true) => {
            let stream_type = response_stream_ident(&method.proto_name);
            quote! {
                struct #svc_name<T: #service_name>(Arc<T>);
                impl<T: #service_name> grpc_server::ServerStreamingService<#input> for #svc_name<T> {
                    type Response = #output;
                    type ResponseStream = T::#stream_type;
                    type Future = BoxFuture<Result<Response<Self::ResponseStream>, Status>>;
                    fn call(&mut self, request: Request<#input>) -> Self::Future {
                        let inner = Arc::clone(&self.0);
                        Box::pin(async move { inner.#method_name(request).await })
                    }
                }
            }
        }
        (true, false) => {
            quote! {
                struct #svc_name<T: #service_name>(Arc<T>);
                impl<T: #service_name> grpc_server::ClientStreamingService<#input> for #svc_name<T> {
                    type Response = #output;
                    type Future = BoxFuture<Result<Response<#output>, Status>>;
                    fn call(&mut self, request: Request<Streaming<#input>>) -> Self::Future {
                        let inner = Arc::clone(&self.0);
                        Box::pin(async move { inner.#method_name(request).await })
                    }
                }
            }
        }
        (true, true) => {
            let stream_type = response_stream_ident(&method.proto_name);
            quote! {
                struct #svc_name<T: #service_name>(Arc<T>);
                impl<T: #service_name> grpc_server::StreamingService<#input> for #svc_name<T> {
                    type Response = #output;
                    type ResponseStream = T::#stream_type;
                    type Future = BoxFuture<Result<Response<Self::ResponseStream>, Status>>;
                    fn call(&mut self, request: Request<Streaming<#input>>) -> Self::Future {
                        let inner = Arc::clone(&self.0);
                        Box::pin(async move { inner.#method_name(request).await })
                    }
                }
            }
        }
    }
}

fn gen_dispatch_arm(method: &MethodDef, service_fqn: &str) -> TokenStream {
    let path = method.grpc_path(service_fqn);
    let svc_name = format_ident!("{}Svc", method.name.to_upper_camel_case());
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

    // Codec type params: <Encode=output, Decode=input> (server encodes output, decodes input)
    let grpc_method = match (method.client_streaming, method.server_streaming) {
        (false, false) => quote! { grpc.unary(#svc_name(inner), req).await },
        (false, true) => quote! { grpc.server_streaming(#svc_name(inner), req).await },
        (true, false) => quote! { grpc.client_streaming(#svc_name(inner), req).await },
        (true, true) => quote! { grpc.streaming(#svc_name(inner), req).await },
    };

    quote! {
        #path => Box::pin(async move {
            let mut grpc = grpc_server::Grpc::new(
                #codec_path::<#output, #input>::default()
            );
            Ok(#grpc_method)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{MethodDef, ServiceDef};

    fn sample_service() -> ServiceDef {
        ServiceDef {
            name: "Greeter".into(),
            package: "helloworld".into(),
            proto_name: "Greeter".into(),
            comments: vec![],
            methods: vec![
                MethodDef {
                    name: "say_hello".into(),
                    proto_name: "SayHello".into(),
                    input_type: "super::HelloRequest".into(),
                    output_type: "super::HelloReply".into(),
                    client_streaming: false,
                    server_streaming: false,
                    codec_path: "grpc_core::codec::prost_codec::ProstCodec".into(),
                    comments: vec![],
                },
                MethodDef {
                    name: "say_hello_stream".into(),
                    proto_name: "SayHelloStream".into(),
                    input_type: "super::HelloRequest".into(),
                    output_type: "super::HelloReply".into(),
                    client_streaming: false,
                    server_streaming: true,
                    codec_path: "grpc_core::codec::prost_codec::ProstCodec".into(),
                    comments: vec![],
                },
            ],
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
    fn generated_code_contains_trait_and_server() {
        let svc = sample_service();
        let tokens = generate(&svc);
        let code = tokens.to_string();
        assert!(code.contains("trait Greeter"), "should contain trait");
        assert!(
            code.contains("GreeterServer"),
            "should contain server struct"
        );
        assert!(code.contains("SayHello"), "should contain method path");
    }

    use crate::test_util::module_items;

    #[test]
    fn generated_trait_has_expected_methods() {
        let svc = sample_service();
        let tokens = generate(&svc);
        let file: syn::File = syn::parse2(tokens).unwrap();
        let items = module_items(&file);

        // Find the trait definition
        let trait_item = items.iter().find_map(|item| {
            if let syn::Item::Trait(t) = item {
                if t.ident == "Greeter" {
                    return Some(t);
                }
            }
            None
        });
        let trait_item = trait_item.expect("should generate a Greeter trait");

        // Collect method names from the trait
        let method_names: Vec<String> = trait_item
            .items
            .iter()
            .filter_map(|item| {
                if let syn::TraitItem::Fn(m) = item {
                    Some(m.sig.ident.to_string())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            method_names.contains(&"say_hello".to_string()),
            "trait should have say_hello method, got: {method_names:?}"
        );
        assert!(
            method_names.contains(&"say_hello_stream".to_string()),
            "trait should have say_hello_stream method, got: {method_names:?}"
        );
    }

    #[test]
    fn generated_server_streaming_has_associated_type() {
        let svc = sample_service();
        let tokens = generate(&svc);
        let code = tokens.to_string();

        // Server-streaming methods should generate an associated type for the response stream
        assert!(
            code.contains("SayHelloResponseStream"),
            "server-streaming method should generate a stream associated type, got: {code}"
        );
    }

    #[test]
    fn generated_server_has_named_service_impl() {
        let svc = sample_service();
        let tokens = generate(&svc);
        let code = tokens.to_string();

        assert!(
            code.contains("NamedService"),
            "server should implement NamedService, got: {code}"
        );
        assert!(
            code.contains("helloworld.Greeter"),
            "NAME should be the fully-qualified service name, got: {code}"
        );
    }

    #[test]
    fn generated_server_has_dispatch_for_all_methods() {
        let svc = sample_service();
        let tokens = generate(&svc);
        let code = tokens.to_string();

        // Each method should have a dispatch arm with its gRPC path
        assert!(
            code.contains("/helloworld.Greeter/SayHello"),
            "should dispatch SayHello, got: {code}"
        );
        assert!(
            code.contains("/helloworld.Greeter/SayHelloStream"),
            "should dispatch SayHelloStream, got: {code}"
        );
    }

    #[test]
    fn generated_server_has_tower_service_impl() {
        let svc = sample_service();
        let tokens = generate(&svc);
        let file: syn::File = syn::parse2(tokens).unwrap();
        let items = module_items(&file);

        // Find impl blocks for GreeterServer
        let server_impls: Vec<_> = items
            .iter()
            .filter_map(|item| {
                if let syn::Item::Impl(imp) = item {
                    let ty = &imp.self_ty;
                    let ty_str = quote::quote!(#ty).to_string();
                    if ty_str.contains("GreeterServer") {
                        return Some(imp);
                    }
                }
                None
            })
            .collect();

        assert!(
            !server_impls.is_empty(),
            "should have impl blocks for GreeterServer"
        );
    }

    #[test]
    fn all_four_rpc_patterns_generate_correct_grpc_calls() {
        let svc = ServiceDef {
            name: "Svc".into(),
            package: "pkg".into(),
            proto_name: "Svc".into(),
            comments: vec![],
            methods: vec![
                MethodDef {
                    name: "unary_m".into(),
                    proto_name: "UnaryM".into(),
                    input_type: "super::Req".into(),
                    output_type: "super::Resp".into(),
                    client_streaming: false,
                    server_streaming: false,
                    codec_path: "Codec".into(),
                    comments: vec![],
                },
                MethodDef {
                    name: "server_m".into(),
                    proto_name: "ServerM".into(),
                    input_type: "super::Req".into(),
                    output_type: "super::Resp".into(),
                    client_streaming: false,
                    server_streaming: true,
                    codec_path: "Codec".into(),
                    comments: vec![],
                },
                MethodDef {
                    name: "client_m".into(),
                    proto_name: "ClientM".into(),
                    input_type: "super::Req".into(),
                    output_type: "super::Resp".into(),
                    client_streaming: true,
                    server_streaming: false,
                    codec_path: "Codec".into(),
                    comments: vec![],
                },
                MethodDef {
                    name: "bidi_m".into(),
                    proto_name: "BidiM".into(),
                    input_type: "super::Req".into(),
                    output_type: "super::Resp".into(),
                    client_streaming: true,
                    server_streaming: true,
                    codec_path: "Codec".into(),
                    comments: vec![],
                },
            ],
        };

        let tokens = generate(&svc);
        let code = tokens.to_string();

        // Verify correct grpc handler methods are called for each pattern
        assert!(
            code.contains("grpc . unary"),
            "unary should call grpc.unary"
        );
        assert!(
            code.contains("grpc . server_streaming"),
            "server_streaming should call grpc.server_streaming"
        );
        assert!(
            code.contains("grpc . client_streaming"),
            "client_streaming should call grpc.client_streaming"
        );
        assert!(
            code.contains("grpc . streaming"),
            "bidi should call grpc.streaming"
        );
    }
}
