//! Tonic server for benchmarking
//!
//! Run with: `cargo run --release -p tonic-bench-server --bin tonic_server -- --port 50053`

use std::net::SocketAddr;
use std::pin::Pin;

use clap::Parser;
use futures::stream::StreamExt;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

const STREAM_COUNT: usize = 1000;

pub mod benchmark_service {
    tonic::include_proto!("benchmark");
}

#[derive(Clone)]
struct BenchmarkServiceImpl;

#[tonic::async_trait]
impl benchmark_service::benchmark_service_server::BenchmarkService for BenchmarkServiceImpl {
    type ServerStreamStream =
        Pin<Box<dyn Stream<Item = Result<benchmark_service::BenchmarkResponse, Status>> + Send>>;
    type BiDiStreamStream =
        Pin<Box<dyn Stream<Item = Result<benchmark_service::BenchmarkResponse, Status>> + Send>>;

    async fn unary_call(
        &self,
        request: Request<benchmark_service::BenchmarkRequest>,
    ) -> Result<Response<benchmark_service::BenchmarkResponse>, Status> {
        let req = request.into_inner();
        Ok(Response::new(benchmark_service::BenchmarkResponse {
            id: req.id,
            payload: req.payload,
            numbers: req.numbers,
        }))
    }

    async fn server_stream(
        &self,
        request: Request<benchmark_service::BenchmarkRequest>,
    ) -> Result<Response<Self::ServerStreamStream>, Status> {
        let req = request.into_inner();

        #[allow(clippy::result_large_err)]
        let stream = tokio_stream::iter(0..STREAM_COUNT).map(move |i| {
            Ok(benchmark_service::BenchmarkResponse {
                id: req.id + i as u64,
                payload: req.payload.clone(),
                numbers: req.numbers.clone(),
            })
        });

        Ok(Response::new(Box::pin(stream) as Self::ServerStreamStream))
    }

    async fn client_stream(
        &self,
        request: Request<tonic::Streaming<benchmark_service::BenchmarkRequest>>,
    ) -> Result<Response<benchmark_service::BenchmarkResponse>, Status> {
        let mut stream = request.into_inner();
        let mut last_id = 0u64;
        let mut total_payload = Vec::new();
        let mut all_numbers = Vec::new();

        while let Some(req) = stream.next().await {
            let req = req?;
            last_id = req.id;
            total_payload.extend_from_slice(req.payload.as_bytes());
            all_numbers.extend_from_slice(&req.numbers);
        }

        Ok(Response::new(benchmark_service::BenchmarkResponse {
            id: last_id,
            payload: String::from_utf8(total_payload).unwrap_or_default(),
            numbers: all_numbers,
        }))
    }

    async fn bi_di_stream(
        &self,
        request: Request<tonic::Streaming<benchmark_service::BenchmarkRequest>>,
    ) -> Result<Response<Self::BiDiStreamStream>, Status> {
        let stream = request.into_inner();

        #[allow(clippy::result_large_err)]
        let stream = stream.map(|result| {
            result.map(|req| benchmark_service::BenchmarkResponse {
                id: req.id,
                payload: req.payload,
                numbers: req.numbers,
            })
        });

        Ok(Response::new(Box::pin(stream) as Self::BiDiStreamStream))
    }
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value_t = 50053)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    let addr: SocketAddr = format!("127.0.0.1:{}", args.port).parse()?;

    println!("Tonic server listening on {}", addr);

    tonic::transport::Server::builder()
        .add_service(
            benchmark_service::benchmark_service_server::BenchmarkServiceServer::new(
                BenchmarkServiceImpl,
            ),
        )
        .serve(addr)
        .await?;

    Ok(())
}
