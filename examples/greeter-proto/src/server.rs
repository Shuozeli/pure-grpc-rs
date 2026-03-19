use greeter_proto::greeter_server::{Greeter, GreeterServer};
use greeter_proto::{HelloReply, HelloRequest};
use grpc_core::{BoxFuture, BoxStream, Request, Response, Status, Streaming};
use grpc_server::{NamedService, Router, Server};
use std::net::SocketAddr;

struct MyGreeter;

impl Greeter for MyGreeter {
    fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<HelloReply>, Status>> {
        let name = request.into_inner().name;
        let reply = HelloReply {
            message: format!("Hello, {name}!"),
        };
        Box::pin(async move { Ok(Response::new(reply)) })
    }

    type SayHelloServerResponseStream = BoxStream<Result<HelloReply, Status>>;

    fn say_hello_server_stream(
        &self,
        request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<Self::SayHelloServerResponseStream>, Status>> {
        let name = request.into_inner().name;
        let stream = tokio_stream::iter((0..3).map(move |i| {
            Ok(HelloReply {
                message: format!("Hello, {name}! (message {i})"),
            })
        }));
        Box::pin(async move { Ok(Response::new(Box::pin(stream) as BoxStream<_>)) })
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
            let reply = HelloReply {
                message: format!("Hello, {}!", names.join(", ")),
            };
            Ok(Response::new(reply))
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
                        message: format!("Hello, {}!", msg.name),
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let addr: SocketAddr = "0.0.0.0:50051".parse()?;
    let greeter = GreeterServer::new(MyGreeter);

    let reflection = grpc_reflection::ReflectionServer::builder()
        .register_encoded_file_descriptor_set(greeter_proto::FILE_DESCRIPTOR_SET)
        .build();

    let router = Router::new()
        .add_service(GreeterServer::<MyGreeter>::NAME, greeter)
        .add_service(grpc_reflection::ReflectionServer::NAME, reflection);

    println!("GreeterServer listening on {addr}");
    println!("Press Ctrl+C to shut down");

    Server::builder()
        .serve_with_shutdown(addr, router, async {
            tokio::signal::ctrl_c().await.ok();
            println!("\nShutting down...");
        })
        .await?;

    Ok(())
}
