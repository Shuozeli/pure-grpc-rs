use greeter_proto::greeter_server::{Greeter, GreeterServer};
use greeter_proto::{HelloReply, HelloRequest};
use grpc_core::{BoxFuture, BoxStream, Request, Response, Status};
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
            message: format!("Hello (TLS), {name}!"),
        };
        Box::pin(async move { Ok(Response::new(reply)) })
    }

    type SayHelloServerStreamResponseStream = BoxStream<Result<HelloReply, Status>>;
    fn say_hello_server_stream(
        &self,
        _request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<Self::SayHelloServerStreamResponseStream>, Status>> {
        Box::pin(async { Err(Status::unimplemented("not implemented in TLS example")) })
    }

    fn say_hello_client_stream(
        &self,
        _request: Request<grpc_core::Streaming<HelloRequest>>,
    ) -> BoxFuture<Result<Response<HelloReply>, Status>> {
        Box::pin(async { Err(Status::unimplemented("not implemented in TLS example")) })
    }

    type SayHelloBidiStreamResponseStream = BoxStream<Result<HelloReply, Status>>;
    fn say_hello_bidi_stream(
        &self,
        _request: Request<grpc_core::Streaming<HelloRequest>>,
    ) -> BoxFuture<Result<Response<Self::SayHelloBidiStreamResponseStream>, Status>> {
        Box::pin(async { Err(Status::unimplemented("not implemented in TLS example")) })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let cert = std::fs::read("examples/greeter-proto/certs/server.crt")?;
    let key = std::fs::read("examples/greeter-proto/certs/server.key")?;

    let addr: SocketAddr = "0.0.0.0:50052".parse()?;
    let greeter = GreeterServer::new(MyGreeter);
    let router = Router::new().add_service(GreeterServer::<MyGreeter>::NAME, greeter);

    println!("GreeterServer (TLS) listening on {addr}");

    Server::builder()
        .tls(&cert, &key)?
        .serve_with_shutdown(addr, router, async {
            tokio::signal::ctrl_c().await.ok();
            println!("\nShutting down...");
        })
        .await?;

    Ok(())
}
