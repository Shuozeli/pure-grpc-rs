//! Integration tests: gRPC over HTTP/3 (QUIC) for all 4 RPC patterns.
#![cfg(feature = "h3")]

use greeter_proto::greeter_client::GreeterClient;
use greeter_proto::greeter_server::{Greeter, GreeterServer};
use greeter_proto::{HelloReply, HelloRequest};
use grpc_client::H3Channel;
use grpc_core::{BoxFuture, BoxStream, Request, Response, Status, Streaming};
use grpc_server::{H3Server, NamedService, Router};
use std::net::SocketAddr;

// --- Test service implementation (same as HTTP/2 tests) ---

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

    type SayHelloServerResponseStream = BoxStream<Result<HelloReply, Status>>;

    fn say_hello_server_stream(
        &self,
        request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<Self::SayHelloServerResponseStream>, Status>> {
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

    type SayHelloBidiResponseStream = BoxStream<Result<HelloReply, Status>>;

    fn say_hello_bidi_stream(
        &self,
        request: Request<Streaming<HelloRequest>>,
    ) -> BoxFuture<Result<Response<Self::SayHelloBidiResponseStream>, Status>> {
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

/// Generate self-signed TLS certificate and key for localhost testing.
fn generate_self_signed_cert() -> (Vec<u8>, Vec<u8>) {
    let subject_alt_names = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
    ];
    let cert = rcgen::generate_simple_self_signed(subject_alt_names).unwrap();
    let cert_pem = cert.cert.pem().into_bytes();
    let key_pem = cert.key_pair.serialize_pem().into_bytes();
    (cert_pem, key_pem)
}

/// Start H3 server on a random UDP port, return (addr, cert_pem).
async fn start_h3_server() -> (SocketAddr, Vec<u8>) {
    let (cert_pem, key_pem) = generate_self_signed_cert();

    // Bind first to get the actual address, then pass the endpoint to serve.
    let endpoint =
        H3Server::bind("127.0.0.1:0".parse().unwrap(), &cert_pem, &key_pem).unwrap();
    let addr = endpoint.local_addr().unwrap();

    let greeter = GreeterServer::new(TestGreeter);
    let router = Router::new().add_service(GreeterServer::<TestGreeter>::NAME, greeter);

    tokio::spawn(async move {
        H3Server::builder()
            .serve_endpoint(endpoint, router)
            .await
            .unwrap();
    });

    // Give the server a moment to start accepting.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    (addr, cert_pem)
}

async fn connect_h3(addr: SocketAddr, ca_pem: &[u8]) -> GreeterClient<H3Channel> {
    // Use 127.0.0.1 directly to avoid IPv6 resolution issues.
    let uri: http::Uri = format!("https://127.0.0.1:{}", addr.port())
        .parse()
        .unwrap();
    let channel = H3Channel::connect(uri, Some(ca_pem)).await.unwrap();
    GreeterClient::new(channel)
}

#[tokio::test]
async fn h3_unary_roundtrip() {
    let (addr, cert_pem) = start_h3_server().await;
    let mut client = connect_h3(addr, &cert_pem).await;

    let resp = client
        .say_hello(HelloRequest {
            name: "h3-test".into(),
        })
        .await
        .unwrap();

    assert_eq!(resp.get_ref().message, "Hello, h3-test!");
}

#[tokio::test]
async fn h3_server_streaming_roundtrip() {
    let (addr, cert_pem) = start_h3_server().await;
    let mut client = connect_h3(addr, &cert_pem).await;

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
async fn h3_client_streaming_roundtrip() {
    let (addr, cert_pem) = start_h3_server().await;
    let mut client = connect_h3(addr, &cert_pem).await;

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
async fn h3_bidi_streaming_roundtrip() {
    let (addr, cert_pem) = start_h3_server().await;
    let mut client = connect_h3(addr, &cert_pem).await;

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
