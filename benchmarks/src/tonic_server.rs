//! Tonic server implementation for benchmarking

use crate::{BenchmarkConfig, BenchmarkData, STREAM_COUNT};
use std::pin::Pin;
use tokio::time::Instant;
use tokio_stream::Stream;
use tonic::{Request, Response, Status, Streaming};

pub mod proto {
    tonic::include_proto!("benchmark");
}

use proto::benchmark_service_server::BenchmarkService;

pub struct TonicServer;

impl TonicServer {
    pub async fn serve(config: BenchmarkConfig) -> Result<(), Box<dyn std::error::Error>> {
        let addr = "127.0.0.1:50051".parse()?;
        println!("Tonic server listening on {}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        loop {
            let (socket, _) = listener.accept().await?;
            let svc = TonicBenchmarkService::new(config.clone());
            let _ = socket;
            // In real benchmark, we'd spawn this
        }
    }
}

#[derive(Clone)]
pub struct TonicBenchmarkService {
    config: BenchmarkConfig,
}

impl TonicBenchmarkService {
    pub fn new(config: BenchmarkConfig) -> Self {
        Self { config }
    }
}

#[tonic::async_trait]
impl BenchmarkService for TonicBenchmarkService {
    async fn unary_call(
        &self,
        request: Request<proto::BenchmarkRequest>,
    ) -> Result<Response<proto::BenchmarkResponse>, Status> {
        let req = request.into_inner();
        Ok(Response::new(proto::BenchmarkResponse {
            id: req.id,
            payload: req.payload,
            numbers: req.numbers,
        }))
    }

    type ServerStreamStream = Pin<Box<dyn Stream<Item = Result<proto::BenchmarkResponse, Status>> + Send>>;

    async fn server_stream(
        &self,
        request: Request<proto::BenchmarkRequest>,
    ) -> Result<Response<Self::ServerStreamStream>, Status> {
        let req = request.into_inner();
        let count = self.config.stream_count;

        let stream = tokio_stream::iter(0..count).map(move |i| {
            Ok(proto::BenchmarkResponse {
                id: req.id + i as u64,
                payload: req.payload.clone(),
                numbers: req.numbers.clone(),
            })
        });

        Ok(Response::new(Box::pin(stream) as Self::ServerStreamStream))
    }

    async fn client_stream(
        &self,
        request: Request<Streaming<proto::BenchmarkRequest>>,
    ) -> Result<Response<proto::BenchmarkResponse>, Status> {
        let mut stream = request.into_inner();
        let mut last_id = 0;
        let mut total_payload = Vec::new();
        let mut all_numbers = Vec::new();

        while let Some(req) = stream.message().await? {
            last_id = req.id;
            total_payload.extend_from_slice(&req.payload);
            all_numbers.extend_from_slice(&req.numbers);
        }

        Ok(Response::new(proto::BenchmarkResponse {
            id: last_id,
            payload: total_payload,
            numbers: all_numbers,
        }))
    }

    type BiDiStreamStream = Pin<Box<dyn Stream<Item = Result<proto::BenchmarkResponse, Status>> + Send>>;

    async fn bi_di_stream(
        &self,
        request: Request<Streaming<proto::BenchmarkRequest>>,
    ) -> Result<Response<Self::BiDiStreamStream>, Status> {
        let stream = request.into_inner();
        let stream = stream.map(|result| {
            result.map(|req| proto::BenchmarkResponse {
                id: req.id,
                payload: req.payload,
                numbers: req.numbers,
            })
        });

        Ok(Response::new(Box::pin(stream) as Self::BiDiStreamStream))
    }
}
