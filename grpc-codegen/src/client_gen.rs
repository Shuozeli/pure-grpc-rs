use crate::ir::{MethodDef, ServiceDef};
use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Generate client code for a service definition.
///
/// Produces an `XxxClient<T>` struct with one async method per RPC.
pub fn generate(service: &ServiceDef) -> TokenStream {
    let client_name = format_ident!("{}Client", service.name);
    let mod_name = format_ident!("{}_client", service.name.to_snake_case());
    let fqn = service.fully_qualified_name();

    let connect_method = gen_connect_method(&client_name);
    let rpc_methods = service.methods.iter().map(|m| gen_rpc_method(m, &fqn));

    quote! {
        pub mod #mod_name {
            use grpc_client::{Grpc, GrpcService};
            use grpc_core::body::Body;
            use grpc_core::codec::Streaming;
            use grpc_core::extensions::GrpcMethod;
            use grpc_core::request::{IntoRequest, IntoStreamingRequest};
            use grpc_core::{Response, Status};
            use http::uri::PathAndQuery;
            use http_body::Body as HttpBody;

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
    let name = format_ident!("{}", method.name);
    let path = method.grpc_path(service_fqn);
    let proto_name = &method.proto_name;
    let codec_path: TokenStream = method.codec_path.parse().unwrap();
    let input: TokenStream = method.input_type.parse().unwrap();
    let output: TokenStream = method.output_type.parse().unwrap();
    let service_fqn_str = service_fqn;

    let ready_check = quote! {
        self.inner.ready().await
            .map_err(|e| Status::unknown(
                format!("Service was not ready: {}", e.into())
            ))?;
    };

    match (method.client_streaming, method.server_streaming) {
        (false, false) => {
            // Unary
            quote! {
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
        (false, true) => {
            // Server streaming
            quote! {
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
        (true, false) => {
            // Client streaming
            quote! {
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
        (true, true) => {
            // Bidi streaming
            quote! {
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
    use crate::ir::{MethodDef, ServiceDef};

    fn sample_service() -> ServiceDef {
        ServiceDef {
            name: "Greeter".into(),
            package: "helloworld".into(),
            proto_name: "Greeter".into(),
            comments: vec![],
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
}
