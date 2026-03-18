//! Integration tests: in-process server + client roundtrip for all 4 RPC patterns.

use greeter_proto::greeter_client::GreeterClient;
use greeter_proto::greeter_server::{Greeter, GreeterServer};
use greeter_proto::{HelloReply, HelloRequest};
use grpc_client::Channel;
use grpc_core::{Request, Response, Status, Streaming};
use grpc_server::{NamedService, Router, Server};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use tokio::net::TcpListener;
use tokio_stream::Stream;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send + 'static>>;

// --- Test service implementation ---

struct TestGreeter;

impl Greeter for TestGreeter {
    fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<HelloReply>, Status>> {
        let name = request.into_inner().name;
        Box::pin(async move {
            Ok(Response::new(HelloReply {
                message: format!("Hello, {name}!"),
            }))
        })
    }

    type SayHelloServerStreamStream = BoxStream<Result<HelloReply, Status>>;

    fn say_hello_server_stream(
        &self,
        request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<Self::SayHelloServerStreamStream>, Status>> {
        let name = request.into_inner().name;
        Box::pin(async move {
            let stream = tokio_stream::iter((0..3).map(move |i| {
                Ok(HelloReply {
                    message: format!("{name}-{i}"),
                })
            }));
            Ok(Response::new(Box::pin(stream) as BoxStream<_>))
        })
    }

    fn say_hello_client_stream(
        &self,
        request: Request<Streaming<HelloRequest>>,
    ) -> BoxFuture<Result<Response<HelloReply>, Status>> {
        Box::pin(async move {
            let mut stream = request.into_inner();
            let mut names = Vec::new();
            while let Some(msg) = stream.message().await? {
                names.push(msg.name);
            }
            Ok(Response::new(HelloReply {
                message: names.join("+"),
            }))
        })
    }

    type SayHelloBidiStreamStream = BoxStream<Result<HelloReply, Status>>;

    fn say_hello_bidi_stream(
        &self,
        request: Request<Streaming<HelloRequest>>,
    ) -> BoxFuture<Result<Response<Self::SayHelloBidiStreamStream>, Status>> {
        Box::pin(async move {
            let mut input = request.into_inner();
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            tokio::spawn(async move {
                while let Some(Ok(msg)) = input.message().await.transpose() {
                    let reply = HelloReply {
                        message: format!("echo:{}", msg.name),
                    };
                    if tx.send(Ok(reply)).await.is_err() {
                        break;
                    }
                }
            });
            let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
            Ok(Response::new(Box::pin(stream) as BoxStream<_>))
        })
    }
}

/// Start server on random port, return the address.
async fn start_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let greeter = GreeterServer::new(TestGreeter);
    let router = Router::new().add_service(GreeterServer::<TestGreeter>::NAME, greeter);

    tokio::spawn(async move {
        Server::builder()
            .serve_with_listener(listener, router)
            .await
            .unwrap();
    });

    // Give server a moment to start accepting
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

async fn connect(addr: SocketAddr) -> GreeterClient<Channel> {
    let uri: http::Uri = format!("http://{addr}").parse().unwrap();
    GreeterClient::connect(uri).await.unwrap()
}

#[tokio::test]
async fn unary_roundtrip() {
    let addr = start_server().await;
    let mut client = connect(addr).await;

    let resp = client
        .say_hello(HelloRequest {
            name: "test".into(),
        })
        .await
        .unwrap();

    assert_eq!(resp.get_ref().message, "Hello, test!");
}

#[tokio::test]
async fn server_streaming_roundtrip() {
    let addr = start_server().await;
    let mut client = connect(addr).await;

    let resp = client
        .say_hello_server_stream(HelloRequest {
            name: "stream".into(),
        })
        .await
        .unwrap();

    let mut stream = resp.into_inner();
    let mut messages = Vec::new();
    while let Some(reply) = stream.message().await.unwrap() {
        messages.push(reply.message);
    }

    assert_eq!(messages, vec!["stream-0", "stream-1", "stream-2"]);
}

#[tokio::test]
async fn client_streaming_roundtrip() {
    let addr = start_server().await;
    let mut client = connect(addr).await;

    let names = vec![
        HelloRequest { name: "A".into() },
        HelloRequest { name: "B".into() },
        HelloRequest { name: "C".into() },
    ];

    let resp = client
        .say_hello_client_stream(tokio_stream::iter(names))
        .await
        .unwrap();

    assert_eq!(resp.get_ref().message, "A+B+C");
}

#[tokio::test]
async fn bidi_streaming_roundtrip() {
    let addr = start_server().await;
    let mut client = connect(addr).await;

    let names = vec![
        HelloRequest { name: "X".into() },
        HelloRequest { name: "Y".into() },
    ];

    let resp = client
        .say_hello_bidi_stream(tokio_stream::iter(names))
        .await
        .unwrap();

    let mut stream = resp.into_inner();
    let mut messages = Vec::new();
    while let Some(reply) = stream.message().await.unwrap() {
        messages.push(reply.message);
    }

    assert_eq!(messages, vec!["echo:X", "echo:Y"]);
}

#[tokio::test]
async fn unimplemented_method_returns_error() {
    let addr = start_server().await;
    let uri: http::Uri = format!("http://{addr}").parse().unwrap();
    let channel = Channel::connect(uri.clone()).await.unwrap();
    let mut grpc = grpc_client::Grpc::with_origin(channel, uri);

    // Call a method that doesn't exist
    let codec = grpc_core::codec::prost_codec::ProstCodec::<HelloRequest, HelloReply>::default();
    let path: http::uri::PathAndQuery = "/helloworld.Greeter/NonExistent".parse().unwrap();
    let req = Request::new(HelloRequest {
        name: "test".into(),
    });

    let result = grpc.unary(req, path, codec).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), grpc_core::Code::Unimplemented);
}

#[tokio::test]
async fn health_check_integration() {
    use grpc_health::{health_service, HealthCheckRequest, HealthCheckResponse, ServingStatus};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let greeter = GreeterServer::new(TestGreeter);
    let (health_svc, health_handle) = health_service();
    health_handle.set_serving("helloworld.Greeter").await;

    let router = Router::new()
        .add_service(GreeterServer::<TestGreeter>::NAME, greeter)
        .add_service("grpc.health.v1.Health", health_svc);

    tokio::spawn(async move {
        Server::builder()
            .serve_with_listener(listener, router)
            .await
            .unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Use raw Grpc client to call health check
    let uri: http::Uri = format!("http://{addr}").parse().unwrap();
    let channel = Channel::connect(uri.clone()).await.unwrap();
    let mut grpc = grpc_client::Grpc::with_origin(channel, uri);

    let codec =
        grpc_core::codec::prost_codec::ProstCodec::<HealthCheckRequest, HealthCheckResponse>::default();
    let path: http::uri::PathAndQuery = "/grpc.health.v1.Health/Check".parse().unwrap();

    // Check overall health (empty service)
    let req = Request::new(HealthCheckRequest {
        service: String::new(),
    });
    let resp = grpc.unary(req, path.clone(), codec).await.unwrap();
    assert_eq!(resp.get_ref().status, ServingStatus::Serving as i32);

    // Check specific service
    let codec =
        grpc_core::codec::prost_codec::ProstCodec::<HealthCheckRequest, HealthCheckResponse>::default();
    let req = Request::new(HealthCheckRequest {
        service: "helloworld.Greeter".into(),
    });
    let resp = grpc.unary(req, path.clone(), codec).await.unwrap();
    assert_eq!(resp.get_ref().status, ServingStatus::Serving as i32);

    // Check unknown service
    let codec =
        grpc_core::codec::prost_codec::ProstCodec::<HealthCheckRequest, HealthCheckResponse>::default();
    let req = Request::new(HealthCheckRequest {
        service: "unknown.Service".into(),
    });
    let resp = grpc.unary(req, path, codec).await.unwrap();
    assert_eq!(resp.get_ref().status, ServingStatus::ServiceUnknown as i32);
}

#[tokio::test]
async fn server_timeout_enforcement() {
    // Create a slow service that sleeps longer than the timeout
    #[derive(Clone)]
    struct SlowService;

    impl tower_service::Service<http::Request<grpc_core::body::Body>> for SlowService {
        type Response = http::Response<grpc_core::body::Body>;
        type Error = std::convert::Infallible;
        type Future = Pin<
            Box<
                dyn Future<
                        Output = Result<
                            http::Response<grpc_core::body::Body>,
                            std::convert::Infallible,
                        >,
                    > + Send,
            >,
        >;

        fn poll_ready(
            &mut self,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: http::Request<grpc_core::body::Body>) -> Self::Future {
            Box::pin(async {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                Ok(http::Response::new(grpc_core::body::Body::empty()))
            })
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        Server::builder()
            .timeout(std::time::Duration::from_millis(100))
            .serve_with_listener(listener, SlowService)
            .await
            .unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut client = connect(addr).await;

    let result = client
        .say_hello(HelloRequest {
            name: "slow".into(),
        })
        .await;

    // Should get DEADLINE_EXCEEDED from the timeout
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err().code(),
        grpc_core::Code::DeadlineExceeded
    );
}

#[tokio::test]
async fn interceptor_rejects_unauthenticated() {
    use grpc_server::InterceptedService;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let greeter = GreeterServer::new(TestGreeter);
    let authed_greeter = InterceptedService::new(greeter, |req: grpc_core::Request<()>| {
        if req.metadata().get("authorization").is_none() {
            return Err(Status::unauthenticated("missing token"));
        }
        Ok(req)
    });

    let router = Router::new().add_service(GreeterServer::<TestGreeter>::NAME, authed_greeter);

    tokio::spawn(async move {
        Server::builder()
            .serve_with_listener(listener, router)
            .await
            .unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Without auth — should be rejected
    let mut client = connect(addr).await;
    let result = client
        .say_hello(HelloRequest {
            name: "no-auth".into(),
        })
        .await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), grpc_core::Code::Unauthenticated);

    // With auth — should succeed
    let mut req = grpc_core::Request::new(HelloRequest {
        name: "authed".into(),
    });
    req.metadata_mut().insert(
        "authorization",
        http::HeaderValue::from_static("Bearer test-token"),
    );
    let resp = client.say_hello(req).await.unwrap();
    assert_eq!(resp.get_ref().message, "Hello, authed!");
}

#[tokio::test]
async fn reflection_lists_services() {
    use grpc_reflection::proto::server_reflection_request;
    use grpc_reflection::proto::server_reflection_response::MessageResponse;
    use grpc_reflection::{ReflectionServer, ServerReflectionRequest, ServerReflectionResponse};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let greeter = GreeterServer::new(TestGreeter);
    let reflection = ReflectionServer::builder()
        .register_encoded_file_descriptor_set(greeter_proto::FILE_DESCRIPTOR_SET)
        .build();

    let router = Router::new()
        .add_service(GreeterServer::<TestGreeter>::NAME, greeter)
        .add_service(ReflectionServer::NAME, reflection);

    tokio::spawn(async move {
        Server::builder()
            .serve_with_listener(listener, router)
            .await
            .unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Use raw streaming client to call reflection
    let uri: http::Uri = format!("http://{addr}").parse().unwrap();
    let channel = Channel::connect(uri.clone()).await.unwrap();
    let mut grpc = grpc_client::Grpc::with_origin(channel, uri);

    // Send list_services request via bidi streaming
    let request_stream = tokio_stream::once(ServerReflectionRequest {
        host: String::new(),
        message_request: Some(server_reflection_request::MessageRequest::ListServices(
            String::new(),
        )),
    });

    let codec = grpc_core::codec::prost_codec::ProstCodec::<
        ServerReflectionRequest,
        ServerReflectionResponse,
    >::default();
    let path: http::uri::PathAndQuery = "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo"
        .parse()
        .unwrap();

    let resp = grpc
        .streaming(grpc_core::Request::new(request_stream), path, codec)
        .await
        .unwrap();

    let mut stream = resp.into_inner();
    let msg = stream.message().await.unwrap().unwrap();

    match msg.message_response {
        Some(MessageResponse::ListServicesResponse(list)) => {
            let names: Vec<_> = list.service.iter().map(|s| s.name.as_str()).collect();
            assert!(
                names.contains(&"helloworld.Greeter"),
                "should list Greeter: {names:?}"
            );
            assert!(
                names.contains(&"grpc.reflection.v1.ServerReflection"),
                "should list reflection: {names:?}"
            );
        }
        other => panic!("expected ListServicesResponse, got {other:?}"),
    }
}
