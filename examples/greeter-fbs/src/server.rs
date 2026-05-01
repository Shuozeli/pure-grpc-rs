use greeter_fbs::greeter_server::{Greeter, GreeterServer};
use greeter_fbs::{HelloReply, HelloRequest};
use grpc_core::{BoxFuture, Request, Response, Status};
use grpc_server::{NamedService, Router, Server};
use std::net::SocketAddr;

struct MyGreeter;

impl Greeter for MyGreeter {
    fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> BoxFuture<Result<Response<HelloReply>, Status>> {
        // The schema declares `name: string;` without `(required)`, so the
        // generated Object API surfaces it as `Option<String>`. Default to
        // an empty greeting when the caller omits the name.
        let name = request.into_inner().name.unwrap_or_default();
        // Owned wrapper types are `#[non_exhaustive]` (mirroring the
        // upstream Object API), so populate via Default + field assignment
        // rather than struct literal -- this remains forward-compatible if
        // schema additions append fields later.
        let mut reply = HelloReply::default();
        reply.message = Some(format!("Hello (FlatBuffers), {name}!"));
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
