use greeter_fbs::greeter_client::GreeterClient;
use greeter_fbs::greeter_server::{Greeter, GreeterServer};
use greeter_fbs::{HelloReply, HelloRequest};
use grpc_core::{BoxFuture, Request, Response, Status};
use grpc_server::{NamedService, Router, Server};
use std::net::SocketAddr;
use tokio::net::TcpListener;

struct TestGreeter;

impl Greeter for TestGreeter {
    fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<HelloReply>, Status>> {
        let name = request.into_inner().name.unwrap_or_default();
        let mut reply = HelloReply::default();
        reply.message = Some(format!("Hello (FlatBuffers), {name}!"));
        Box::pin(async move { Ok(Response::new(reply)) })
    }
}

async fn wait_for_server(addr: SocketAddr) {
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("server at {addr} did not become ready within 2s");
}

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

    wait_for_server(addr).await;
    addr
}

fn make_request(name: &str) -> HelloRequest {
    let mut req = HelloRequest::default();
    req.name = Some(name.into());
    req
}

#[tokio::test]
async fn flatbuffers_unary_roundtrip() {
    let addr = start_server().await;
    let mut client = connect(addr).await;

    let resp = client.say_hello(make_request("FBS-test")).await.unwrap();

    assert_eq!(
        resp.get_ref().message.as_deref(),
        Some("Hello (FlatBuffers), FBS-test!")
    );
}

#[tokio::test]
async fn flatbuffers_unimplemented_method() {
    let addr = start_server().await;
    let uri: http::Uri = format!("http://{addr}").parse().unwrap();
    let channel = grpc_client::Channel::connect(uri.clone()).await.unwrap();
    let mut grpc = grpc_client::Grpc::with_origin(channel, uri);

    let codec = grpc_codec_flatbuffers::FlatBuffersCodec::<HelloRequest, HelloReply>::default();
    let path: http::uri::PathAndQuery = "/helloworld.Greeter/NonExistent".parse().unwrap();
    let req = Request::new(make_request("test"));

    let result = grpc.unary(req, path, codec).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), grpc_core::Code::Unimplemented);
}

#[tokio::test]
async fn flatbuffers_metadata_roundtrip() {
    let addr = start_server().await;
    let mut client = connect(addr).await;

    let mut req = Request::new(make_request("meta-test"));
    req.metadata_mut().insert(
        "x-custom-header",
        http::HeaderValue::from_static("custom-value"),
    );

    let resp = client.say_hello(req).await.unwrap();
    assert_eq!(
        resp.get_ref().message.as_deref(),
        Some("Hello (FlatBuffers), meta-test!")
    );
}

#[tokio::test]
async fn flatbuffers_concurrent_requests() {
    let addr = start_server().await;

    let mut handles = Vec::new();
    for i in 0..5 {
        handles.push(tokio::spawn(async move {
            let mut client = connect(addr).await;
            let resp = client
                .say_hello(make_request(&format!("client-{i}")))
                .await
                .unwrap();
            resp.get_ref().message.clone()
        }));
    }

    for (i, handle) in handles.into_iter().enumerate() {
        let msg = handle.await.unwrap();
        assert_eq!(msg, Some(format!("Hello (FlatBuffers), client-{i}!")));
    }
}

async fn connect(addr: SocketAddr) -> GreeterClient<grpc_client::Channel> {
    let uri: http::Uri = format!("http://{addr}").parse().unwrap();
    GreeterClient::connect(uri).await.unwrap()
}
