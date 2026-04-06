//! pure-grpc server (FlatBuffers) for benchmarking

use crate::{BenchmarkConfig, STREAM_COUNT};
use grpc_server::{NamedService, Server, Service};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::Request as HyperRequest;
use hyper::Response as HyperResponse;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::net::TcpListener;

pub mod fbs {
    include!(concat!(env!("OUT_DIR"), "/benchmark_generated.rs"));
}

pub struct PureGrpcServer {
    config: BenchmarkConfig,
}

impl PureGrpcServer {
    pub fn new(config: BenchmarkConfig) -> Self {
        Self { config }
    }

    pub async fn serve(&self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(addr).await?;
        println!("pure-grpc server listening on {}", addr);

        loop {
            let (socket, _) = listener.accept().await?;
            let _ = socket;
            // In real benchmark, we'd spawn this
        }
    }
}

pub struct BenchmarkServiceImpl {
    config: BenchmarkConfig,
}

impl BenchmarkServiceImpl {
    pub fn new(config: BenchmarkConfig) -> Self {
        Self { config }
    }
}

impl NamedService for BenchmarkServiceImpl {
    const NAME: &'static str = "/benchmark.BenchmarkService";
}
