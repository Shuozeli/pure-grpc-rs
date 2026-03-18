use greeter_fbs::greeter_server::{Greeter, GreeterServer};
use greeter_fbs::{HelloReply, HelloRequest};
use grpc_core::{Request, Response, Status};
use grpc_server::{NamedService, Router, Server};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

struct MyGreeter;

impl Greeter for MyGreeter {
    fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<HelloReply>, Status>> {
        let name = request.into_inner().name;
        let reply = HelloReply {
            message: format!("Hello (FlatBuffers), {name}!"),
        };
        Box::pin(async move { Ok(Response::new(reply)) })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let addr: SocketAddr = "0.0.0.0:50053".parse()?;
    let greeter = GreeterServer::new(MyGreeter);
    let router = Router::new().add_service(GreeterServer::<MyGreeter>::NAME, greeter);

    println!("FlatBuffers GreeterServer listening on {addr}");

    Server::builder()
        .serve_with_shutdown(addr, router, async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;

    Ok(())
}
