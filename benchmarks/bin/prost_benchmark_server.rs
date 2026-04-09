//! Prost-based Benchmark Service server using pure-grpc-rs
//!
//! Run with: `cargo run --release -p benchmarks --bin prost_benchmark_server -- --port 50052`

use std::net::SocketAddr;
use std::pin::Pin;

use clap::Parser;
use tokio_stream::Stream;
use tokio_stream::StreamExt;

// Include generated proto code
#[allow(clippy::all, unused, dead_code, unused_imports)]
#[allow(non_snake_case)]
mod generated {
    #![allow(clippy::all, unused, dead_code, unused_imports)]
    include!(concat!(env!("OUT_DIR"), "/benchmark.rs"));
}

use generated::benchmark_service_server::{BenchmarkService, BenchmarkServiceServer};
use generated::{BenchmarkRequest, BenchmarkResponse};
use grpc_server::NamedService;
use grpc_core::{Request, Response, Status, Streaming};

const STREAM_COUNT: usize = 1000;

#[derive(Clone)]
struct BenchmarkServiceImpl;

impl BenchmarkService for BenchmarkServiceImpl {
    type ServerResponseStream = Pin<Box<dyn Stream<Item = Result<BenchmarkResponse, Status>> + Send>>;
    type BiDiResponseStream = Pin<Box<dyn Stream<Item = Result<BenchmarkResponse, Status>> + Send>>;

    fn unary_call(
        &self,
        request: Request<BenchmarkRequest>,
    ) -> Pin<Box<dyn futures::Future<Output = Result<Response<BenchmarkResponse>, Status>> + Send>>
    {
        let req = request.into_inner();
        Box::pin(async move {
            Ok(Response::new(BenchmarkResponse {
                id: req.id,
                payload: req.payload,
                numbers: req.numbers,
            }))
        })
    }

    fn server_stream(
        &self,
        request: Request<BenchmarkRequest>,
    ) -> Pin<Box<dyn futures::Future<Output = Result<Response<Self::ServerResponseStream>, Status>> + Send>>
    {
        let req = request.into_inner();
        let payload = req.payload.clone();
        let numbers = req.numbers.clone();
        let base_id = req.id;

        Box::pin(async move {
            let stream = tokio_stream::iter(0..STREAM_COUNT as u64).map(move |i| {
                Ok(BenchmarkResponse {
                    id: base_id + i,
                    payload: payload.clone(),
                    numbers: numbers.clone(),
                })
            });
            Ok(Response::new(Box::pin(stream) as Self::ServerResponseStream))
        })
    }

    fn client_stream(
        &self,
        request: Request<Streaming<BenchmarkRequest>>,
    ) -> Pin<Box<dyn futures::Future<Output = Result<Response<BenchmarkResponse>, Status>> + Send>>
    {
        Box::pin(async move {
            let mut stream = request.into_inner();
            let mut last_id = 0u64;
            let mut total_payload = Vec::new();
            let mut all_numbers = Vec::new();

            loop {
                match stream.message().await {
                    Ok(Some(req)) => {
                        last_id = req.id;
                        total_payload.extend_from_slice(req.payload.as_bytes());
                        all_numbers.extend_from_slice(&req.numbers);
                    }
                    Ok(None) => break,
                    Err(e) => return Err(e),
                }
            }

            Ok(Response::new(BenchmarkResponse {
                id: last_id,
                payload: String::from_utf8(total_payload).unwrap_or_default(),
                numbers: all_numbers,
            }))
        })
    }

    fn bi_di_stream(
        &self,
        request: Request<Streaming<BenchmarkRequest>>,
    ) -> Pin<Box<dyn futures::Future<Output = Result<Response<Self::BiDiResponseStream>, Status>> + Send>>
    {
        let stream = request.into_inner();

        Box::pin(async move {
            let stream = stream.map(|result| {
                result.map(|req| BenchmarkResponse {
                    id: req.id,
                    payload: req.payload,
                    numbers: req.numbers,
                })
            });
            Ok(Response::new(Box::pin(stream) as Self::BiDiResponseStream))
        })
    }
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value_t = 50052)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    let addr: SocketAddr = format!("127.0.0.1:{}", args.port).parse()?;

    let benchmark_service = BenchmarkServiceServer::new(BenchmarkServiceImpl);

    let router = grpc_server::Router::new()
        .add_service(BenchmarkServiceServer::<BenchmarkServiceImpl>::NAME, benchmark_service);

    println!("Prost Benchmark server listening on port {}", args.port);

    grpc_server::Server::builder()
        .serve_with_shutdown(addr, router, async {
            tokio::signal::ctrl_c().await.ok();
            println!("\nShutting down...");
        })
        .await?;

    Ok(())
}