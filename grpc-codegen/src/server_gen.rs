use crate::ir::{MethodDef, ServiceDef};
use heck::{ToSnakeCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Generate server code for a service definition.
///
/// Produces:
/// - A trait with one method per RPC
/// - An `XxxServer<T>` wrapper struct implementing `tower::Service`
/// - Per-method service structs bridging the trait to UnaryService/etc.
pub fn generate(service: &ServiceDef) -> TokenStream {
    let service_name = format_ident!("{}", service.name);
    let server_name = format_ident!("{}Server", service.name);
    let mod_name = format_ident!("{}_server", service.name.to_snake_case());
    let fqn = service.fully_qualified_name();

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
    let dispatch_arms = service
        .methods
        .iter()
        .map(|m| gen_dispatch_arm(m, &fqn, &service_name));

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

            impl<T: #service_name> tower_service::Service<http::Request<Body>> for #server_name<T> {
                type Response = http::Response<Body>;
                type Error = Infallible;
                type Future = BoxFuture<Result<http::Response<Body>, Infallible>>;

                fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
                    Poll::Ready(Ok(()))
                }

                fn call(&mut self, req: http::Request<Body>) -> Self::Future {
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
    let input: TokenStream = method.input_type.parse().unwrap();
    let output: TokenStream = method.output_type.parse().unwrap();

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
            let stream_type = format_ident!("{}Stream", method.name.to_upper_camel_case());
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
            let stream_type = format_ident!("{}Stream", method.name.to_upper_camel_case());
            quote! {
                fn #name(&self, request: Request<Streaming<#input>>)
                    -> BoxFuture<Result<Response<Self::#stream_type>, Status>>;
            }
        }
    }
}

fn gen_trait_assoc_type(method: &MethodDef) -> TokenStream {
    let stream_type = format_ident!("{}Stream", method.name.to_upper_camel_case());
    let output: TokenStream = method.output_type.parse().unwrap();
    quote! {
        type #stream_type: Stream<Item = Result<#output, Status>> + Send + 'static;
    }
}

fn gen_svc_struct(method: &MethodDef, service_name: &proc_macro2::Ident) -> TokenStream {
    let svc_name = format_ident!("{}Svc", method.name.to_upper_camel_case());
    let method_name = format_ident!("{}", method.name);
    let input: TokenStream = method.input_type.parse().unwrap();
    let output: TokenStream = method.output_type.parse().unwrap();

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
            let stream_type = format_ident!("{}Stream", method.name.to_upper_camel_case());
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
            let stream_type = format_ident!("{}Stream", method.name.to_upper_camel_case());
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

fn gen_dispatch_arm(
    method: &MethodDef,
    service_fqn: &str,
    _service_name: &proc_macro2::Ident,
) -> TokenStream {
    let path = method.grpc_path(service_fqn);
    let svc_name = format_ident!("{}Svc", method.name.to_upper_camel_case());
    let codec_path: TokenStream = method.codec_path.parse().unwrap();
    let input: TokenStream = method.input_type.parse().unwrap();
    let output: TokenStream = method.output_type.parse().unwrap();

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
}
