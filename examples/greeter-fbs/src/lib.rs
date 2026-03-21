//! FlatBuffers greeter -- generated types from `schema/greeter.fbs` via grpc-build,
//! with hand-written FlatBufferGrpcMessage impls for gRPC codec bridging.

// Include the generated FlatBuffers readers/builders
#[allow(
    unused_imports,
    dead_code,
    non_snake_case,
    clippy::extra_unused_lifetimes,
    clippy::needless_lifetimes,
    clippy::derivable_impls,
    clippy::unnecessary_cast
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/greeter_generated.rs"));
}

use grpc_codec_flatbuffers::FlatBufferGrpcMessage;

// --- Owned message types that implement FlatBufferGrpcMessage ---
//
// The generated FlatBuffers types are zero-copy (lifetime-bound), so we need
// owned wrappers for gRPC's Send + 'static requirement. These use the generated
// builders for encoding and readers for decoding.

#[derive(Debug, Clone, PartialEq)]
pub struct HelloRequest {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HelloReply {
    pub message: String,
}

impl FlatBufferGrpcMessage for HelloRequest {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let name = builder.create_string(&self.name);
        let req = generated::helloworld::HelloRequest::create(
            &mut builder,
            &generated::helloworld::HelloRequestArgs { name: Some(name) },
        );
        builder.finish(req, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let req = flatbuffers::root::<generated::helloworld::HelloRequest>(data)
            .map_err(|e| format!("invalid HelloRequest: {e}"))?;
        Ok(HelloRequest {
            name: req
                .name()
                .ok_or("HelloRequest.name is required")?
                .to_string(),
        })
    }
}

impl FlatBufferGrpcMessage for HelloReply {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let message = builder.create_string(&self.message);
        let reply = generated::helloworld::HelloReply::create(
            &mut builder,
            &generated::helloworld::HelloReplyArgs {
                message: Some(message),
            },
        );
        builder.finish(reply, None);
        builder.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let reply = flatbuffers::root::<generated::helloworld::HelloReply>(data)
            .map_err(|e| format!("invalid HelloReply: {e}"))?;
        Ok(HelloReply {
            message: reply
                .message()
                .ok_or("HelloReply.message is required")?
                .to_string(),
        })
    }
}

// --- Server stubs ---

pub mod greeter_server {
    use super::{HelloReply, HelloRequest};
    use grpc_codec_flatbuffers::FlatBuffersCodec;
    use grpc_core::body::Body;
    use grpc_core::{BoxFuture, Request, Response, Status};
    use grpc_server::{Grpc, NamedService, UnaryService};
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::task::{Context, Poll};

    pub trait Greeter: Send + Sync + 'static {
        fn say_hello(
            &self,
            request: Request<HelloRequest>,
        ) -> BoxFuture<Result<Response<HelloReply>, Status>>;
    }

    pub struct GreeterServer<T> {
        inner: Arc<T>,
    }

    impl<T> Clone for GreeterServer<T> {
        fn clone(&self) -> Self {
            Self {
                inner: Arc::clone(&self.inner),
            }
        }
    }

    impl<T: Greeter> GreeterServer<T> {
        pub fn new(inner: T) -> Self {
            Self {
                inner: Arc::new(inner),
            }
        }
    }

    impl<T: Greeter> NamedService for GreeterServer<T> {
        const NAME: &'static str = "helloworld.Greeter";
    }

    struct SayHelloSvc<T: Greeter>(Arc<T>);

    impl<T: Greeter> UnaryService<HelloRequest> for SayHelloSvc<T> {
        type Response = HelloReply;
        type Future = BoxFuture<Result<Response<HelloReply>, Status>>;
        fn call(&mut self, request: Request<HelloRequest>) -> Self::Future {
            let inner = Arc::clone(&self.0);
            Box::pin(async move { inner.say_hello(request).await })
        }
    }

    impl<T: Greeter> tower_service::Service<http::Request<Body>> for GreeterServer<T> {
        type Response = http::Response<Body>;
        type Error = Infallible;
        type Future = BoxFuture<Result<http::Response<Body>, Infallible>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: http::Request<Body>) -> Self::Future {
            let inner = self.inner.clone();
            match req.uri().path() {
                "/helloworld.Greeter/SayHello" => Box::pin(async move {
                    let mut grpc =
                        Grpc::new(FlatBuffersCodec::<HelloReply, HelloRequest>::default());
                    Ok(grpc.unary(SayHelloSvc(inner), req).await)
                }),
                _ => Box::pin(async { Ok(Status::unimplemented("").into_http()) }),
            }
        }
    }
}

// --- Client stubs ---

pub mod greeter_client {
    use super::{HelloReply, HelloRequest};
    use grpc_client::{Grpc, GrpcService};
    use grpc_codec_flatbuffers::FlatBuffersCodec;
    use grpc_core::body::Body;
    use grpc_core::extensions::GrpcMethod;
    use grpc_core::request::IntoRequest;
    use grpc_core::{Response, Status};
    use http::uri::PathAndQuery;
    use http_body::Body as HttpBody;

    #[derive(Debug, Clone)]
    pub struct GreeterClient<T> {
        inner: Grpc<T>,
    }

    impl GreeterClient<grpc_client::Channel> {
        pub async fn connect(
            uri: http::Uri,
        ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
            let channel = grpc_client::Channel::connect(uri.clone()).await?;
            Ok(Self {
                inner: Grpc::with_origin(channel, uri),
            })
        }
    }

    impl<T> GreeterClient<T>
    where
        T: GrpcService<Body>,
        T::Error: Into<grpc_core::BoxError>,
        T::ResponseBody: HttpBody<Data = bytes::Bytes> + Send + 'static,
        <T::ResponseBody as HttpBody>::Error: Into<grpc_core::BoxError>,
    {
        pub async fn say_hello(
            &mut self,
            request: impl IntoRequest<HelloRequest>,
        ) -> Result<Response<HelloReply>, Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| Status::unknown(format!("Service was not ready: {}", e.into())))?;
            let codec = FlatBuffersCodec::<HelloRequest, HelloReply>::default();
            let path = PathAndQuery::from_static("/helloworld.Greeter/SayHello");
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("helloworld.Greeter", "SayHello"));
            self.inner.unary(req, path, codec).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_request_roundtrip() {
        let req = HelloRequest {
            name: "FlatBuffers".into(),
        };
        let decoded = HelloRequest::decode_flatbuffer(&req.encode_flatbuffer()).unwrap();
        assert_eq!(decoded.name, "FlatBuffers");
    }

    #[test]
    fn hello_reply_roundtrip() {
        let reply = HelloReply {
            message: "Hello!".into(),
        };
        let decoded = HelloReply::decode_flatbuffer(&reply.encode_flatbuffer()).unwrap();
        assert_eq!(decoded.message, "Hello!");
    }

    #[test]
    fn decode_invalid_data() {
        assert!(HelloRequest::decode_flatbuffer(&[0, 0]).is_err());
    }
}
