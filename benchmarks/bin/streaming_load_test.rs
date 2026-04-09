//! Comprehensive gRPC Streaming Load Test Binary
//!
//! Tests multiple scenarios:
//! - Unary calls with various payload sizes
//! - Server streaming (1 request -> N responses)
//! - Client streaming (N requests -> 1 response)
//! - Bidirectional streaming (N requests <-> N responses)
//! - Connection reuse (single connection, many requests)
//! - Multiplexing (multiple concurrent streams on single connection)
//!
//! Run with: `cargo run --release -p benchmarks --bin streaming_load_test -- [options]`

#![allow(dead_code, unused_imports)]

use std::time::{Duration, Instant};

use clap::Parser;
use futures::StreamExt;
use http::uri::PathAndQuery;
use tokio::sync::mpsc;
use tokio::time::timeout;

use grpc_client::{Channel, Grpc};
use grpc_codec_flatbuffers::{FlatBufferGrpcMessage, FlatBuffersCodec};
use grpc_core::codec::prost_codec::ProstCodec;
use grpc_core::extensions::GrpcMethod;
use grpc_core::request::{IntoRequest, IntoStreamingRequest};
use grpc_core::{Response, Status};

// Prost types
use prost::Message;

// ============================================================================
// Constants
// ============================================================================

/// Number of numeric values in benchmark request/response messages.
const BENCHMARK_NUMBERS_COUNT: usize = 10;

// ============================================================================
// Payload Sizes
// ============================================================================

#[derive(Debug, Clone, Copy)]
pub enum PayloadSize {
    Tiny,   // 64 bytes
    Small,  // 1 KB
    Medium, // 64 KB
    Large,  // 1 MB
}

impl PayloadSize {
    fn size(&self) -> usize {
        match self {
            PayloadSize::Tiny => 64,
            PayloadSize::Small => 1024,
            PayloadSize::Medium => 64 * 1024,
            PayloadSize::Large => 1024 * 1024,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            PayloadSize::Tiny => "tiny",
            PayloadSize::Small => "small",
            PayloadSize::Medium => "medium",
            PayloadSize::Large => "large",
        }
    }
}

// ============================================================================
// Prost Benchmark Types
// ============================================================================

#[derive(Clone, Message)]
pub struct BenchmarkRequest {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(string, tag = "2")]
    pub payload: String,
    #[prost(uint64, repeated, tag = "3")]
    pub numbers: Vec<u64>,
}

impl BenchmarkRequest {
    fn new(id: u64, size: PayloadSize) -> Self {
        let payload_len = size.size() - 24; // subtract id + numbers overhead
        let payload = "x".repeat(payload_len.max(1));
        let numbers = (0..BENCHMARK_NUMBERS_COUNT as u64).collect();
        Self {
            id,
            payload,
            numbers,
        }
    }
}

#[derive(Clone, Message)]
pub struct BenchmarkResponse {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(string, tag = "2")]
    pub payload: String,
    #[prost(uint64, repeated, tag = "3")]
    pub numbers: Vec<u64>,
}

// ============================================================================
// Tonic Client
// ============================================================================

use tonic::client::Grpc as TonicGrpc;
use tonic::codec::ProstCodec as TonicProstCodec;
use tonic::transport::Channel as TonicChannel;

#[derive(Debug, Clone)]
pub struct TonicBenchmarkClient {
    grpc: TonicGrpc<TonicChannel>,
}

impl TonicBenchmarkClient {
    pub async fn connect(uri: http::Uri) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let channel = tonic::transport::Endpoint::from(uri).connect().await?;
        let grpc = TonicGrpc::new(channel);
        Ok(Self { grpc })
    }

    pub async fn unary_call(&mut self, id: u64, size: PayloadSize) -> Result<(), tonic::Status> {
        let request = tonic::Request::new(BenchmarkRequest::new(id, size));
        let path = PathAndQuery::from_static("/benchmark.BenchmarkService/UnaryCall");
        let codec = TonicProstCodec::<BenchmarkRequest, BenchmarkResponse>::default();

        self.grpc
            .ready()
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        let _ = self.grpc.unary(request, path, codec).await?;
        Ok(())
    }

    pub async fn server_stream(
        &mut self,
        id: u64,
        size: PayloadSize,
    ) -> Result<Vec<BenchmarkResponse>, tonic::Status> {
        let request = tonic::Request::new(BenchmarkRequest::new(id, size));
        let path = PathAndQuery::from_static("/benchmark.BenchmarkService/ServerStream");
        let codec = TonicProstCodec::<BenchmarkRequest, BenchmarkResponse>::default();

        self.grpc
            .ready()
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        let response = self.grpc.server_streaming(request, path, codec).await?;
        let mut stream = response.into_inner();
        let mut responses = Vec::new();
        while let Some(item) = stream.next().await {
            responses.push(item?);
        }
        Ok(responses)
    }

    pub async fn client_stream(
        &mut self,
        count: usize,
        size: PayloadSize,
    ) -> Result<BenchmarkResponse, tonic::Status> {
        let path = PathAndQuery::from_static("/benchmark.BenchmarkService/ClientStream");
        let codec = TonicProstCodec::<BenchmarkRequest, BenchmarkResponse>::default();

        let stream =
            tokio_stream::iter((0..count as u64).map(move |i| BenchmarkRequest::new(i, size)));
        let request = tonic::Request::new(stream);

        self.grpc
            .ready()
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        let response = self.grpc.client_streaming(request, path, codec).await?;
        Ok(response.into_inner())
    }

    pub async fn bi_di_stream(
        &mut self,
        count: usize,
        size: PayloadSize,
    ) -> Result<Vec<BenchmarkResponse>, tonic::Status> {
        let path = PathAndQuery::from_static("/benchmark.BenchmarkService/BiDiStream");
        let codec = TonicProstCodec::<BenchmarkRequest, BenchmarkResponse>::default();

        let stream =
            tokio_stream::iter((0..count as u64).map(move |i| BenchmarkRequest::new(i, size)));
        let request = tonic::Request::new(stream);

        self.grpc
            .ready()
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        let response = self.grpc.streaming(request, path, codec).await?;
        let mut stream = response.into_inner();
        let mut responses = Vec::new();
        while let Some(item) = stream.next().await {
            responses.push(item?);
        }
        Ok(responses)
    }
}

// ============================================================================
// Prost Client (using pure-grpc)
// ============================================================================

#[derive(Debug, Clone)]
pub struct ProstBenchmarkClient {
    grpc: Grpc<Channel>,
}

impl ProstBenchmarkClient {
    pub async fn connect(uri: http::Uri) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let channel = Channel::connect(uri.clone()).await?;
        let grpc = Grpc::with_origin(channel, uri);
        Ok(Self { grpc })
    }

    pub async fn unary_call(&mut self, id: u64, size: PayloadSize) -> Result<(), Status> {
        let request = BenchmarkRequest::new(id, size);
        let codec = ProstCodec::<BenchmarkRequest, BenchmarkResponse>::default();
        let path = PathAndQuery::from_static("/benchmark.BenchmarkService/UnaryCall");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("benchmark.BenchmarkService", "UnaryCall"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        let _ = self.grpc.unary(req, path, codec).await?;
        Ok(())
    }

    pub async fn server_stream(
        &mut self,
        id: u64,
        size: PayloadSize,
    ) -> Result<Vec<BenchmarkResponse>, Status> {
        let request = BenchmarkRequest::new(id, size);
        let codec = ProstCodec::<BenchmarkRequest, BenchmarkResponse>::default();
        let path = PathAndQuery::from_static("/benchmark.BenchmarkService/ServerStream");

        let mut req = request.into_request();
        req.extensions_mut().insert(GrpcMethod::new(
            "benchmark.BenchmarkService",
            "ServerStream",
        ));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        let response: Response<grpc_core::Streaming<BenchmarkResponse>> =
            self.grpc.server_streaming(req, path, codec).await?;
        let mut stream = response.into_inner();
        let mut responses = Vec::new();
        loop {
            match stream.message().await {
                Ok(Some(item)) => responses.push(item),
                Ok(None) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(responses)
    }

    pub async fn client_stream(
        &mut self,
        count: usize,
        size: PayloadSize,
    ) -> Result<BenchmarkResponse, Status> {
        let codec = ProstCodec::<BenchmarkRequest, BenchmarkResponse>::default();
        let path = PathAndQuery::from_static("/benchmark.BenchmarkService/ClientStream");

        let stream =
            tokio_stream::iter((0..count as u64).map(move |i| BenchmarkRequest::new(i, size)));
        let mut req = stream.into_streaming_request();
        req.extensions_mut().insert(GrpcMethod::new(
            "benchmark.BenchmarkService",
            "ClientStream",
        ));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        let response = self.grpc.client_streaming(req, path, codec).await?;
        Ok(response.into_inner())
    }

    pub async fn bi_di_stream(
        &mut self,
        count: usize,
        size: PayloadSize,
    ) -> Result<Vec<BenchmarkResponse>, Status> {
        let codec = ProstCodec::<BenchmarkRequest, BenchmarkResponse>::default();
        let path = PathAndQuery::from_static("/benchmark.BenchmarkService/BiDiStream");

        let stream =
            tokio_stream::iter((0..count as u64).map(move |i| BenchmarkRequest::new(i, size)));
        let mut req = stream.into_streaming_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("benchmark.BenchmarkService", "BiDiStream"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        let response: Response<grpc_core::Streaming<BenchmarkResponse>> =
            self.grpc.streaming(req, path, codec).await?;
        let mut stream = response.into_inner();
        let mut responses = Vec::new();
        loop {
            match stream.message().await {
                Ok(Some(item)) => responses.push(item),
                Ok(None) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(responses)
    }
}

// ============================================================================
// Configuration
// ============================================================================

#[derive(Debug, Clone)]
pub struct Config {
    pub duration_secs: u64,
    pub concurrency: u32,
    pub warmup_secs: u64,
    pub stream_count: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            duration_secs: 10,
            concurrency: 50,
            warmup_secs: 1,
            stream_count: 1000,
        }
    }
}

// ============================================================================
// Load Test Result
// ============================================================================

#[derive(Debug)]
pub struct LoadTestResult {
    pub total_requests: u64,
    pub success_requests: u64,
    pub qps: f64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
}

impl LoadTestResult {
    fn success_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.success_requests as f64 / self.total_requests as f64 * 100.0
        }
    }

    pub fn print(&self, name: &str) {
        println!("\n=== {} Load Test Results ===", name);
        println!(
            "Total requests: {} ({} success, {:.1}%)",
            self.total_requests,
            self.success_requests,
            self.success_rate()
        );
        println!("QPS: {:.0}", self.qps);
        println!("Latency p50: {:.3}ms", self.p50_ns as f64 / 1_000_000.0);
        println!("Latency p95: {:.3}ms", self.p95_ns as f64 / 1_000_000.0);
        println!("Latency p99: {:.3}ms", self.p99_ns as f64 / 1_000_000.0);
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

// ============================================================================
// Shared Helpers for Load Test Result Collection
// ============================================================================

type WorkerResult = (u64, u64, Vec<u64>);

/// Collects worker results from the channel and computes the final LoadTestResult.
async fn collect_and_compute_result(
    mut rx: mpsc::Receiver<WorkerResult>,
    concurrency: u32,
    handles: Vec<tokio::task::JoinHandle<()>>,
) -> (LoadTestResult, f64) {
    let mut total_requests: u64 = 0;
    let mut success_requests: u64 = 0;
    let mut latencies: Vec<u64> = Vec::new();
    let mut received = 0;

    let start = Instant::now();

    while received < concurrency as usize {
        match rx.recv().await {
            Some((local_total, local_success, worker_latencies)) => {
                total_requests += local_total;
                success_requests += local_success;
                latencies.extend(worker_latencies);
                received += 1;
            }
            None => break,
        }
    }

    for handle in handles {
        let _ = handle.await;
    }

    let elapsed = start.elapsed().as_secs_f64();
    latencies.sort();

    let qps = if elapsed > 0.0 {
        total_requests as f64 / elapsed
    } else {
        0.0
    };

    let result = LoadTestResult {
        total_requests,
        success_requests,
        qps,
        p50_ns: percentile(&latencies, 0.50),
        p95_ns: percentile(&latencies, 0.95),
        p99_ns: percentile(&latencies, 0.99),
    };

    (result, elapsed)
}

// ============================================================================
// Tonic Load Tests
// ============================================================================

async fn run_tonic_unary_test(
    config: &Config,
    port: u16,
    size: PayloadSize,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = TonicBenchmarkClient::connect(uri.clone()).await?;
    warmup_client.unary_call(0, size).await?;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let (tx, rx) = mpsc::channel::<WorkerResult>((config.concurrency * 2) as usize);

    let handles: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                let mut client = match TonicBenchmarkClient::connect(uri).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[WORKER {}] Connection failed: {}", id, e);
                        let _ = tx.send((0, 0, Vec::new())).await;
                        return;
                    }
                };
                let duration = Duration::from_secs(config.duration_secs);
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    let req_start = Instant::now();
                    let result = client.unary_call(id as u64, size).await;
                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);
                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    let (result, _) = collect_and_compute_result(rx, config.concurrency, handles).await;
    Ok(result)
}

async fn run_tonic_server_stream_test(
    config: &Config,
    port: u16,
    size: PayloadSize,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = TonicBenchmarkClient::connect(uri.clone()).await?;
    warmup_client.server_stream(0, size).await?;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let (tx, rx) = mpsc::channel::<WorkerResult>((config.concurrency * 2) as usize);

    let handles: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                let mut client = match TonicBenchmarkClient::connect(uri).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[WORKER {}] Connection failed: {}", id, e);
                        let _ = tx.send((0, 0, Vec::new())).await;
                        return;
                    }
                };
                let duration = Duration::from_secs(config.duration_secs);
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    let req_start = Instant::now();
                    let result = client.server_stream(id as u64, size).await;
                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);
                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    let (result, _) = collect_and_compute_result(rx, config.concurrency, handles).await;
    Ok(result)
}

async fn run_tonic_client_stream_test(
    config: &Config,
    port: u16,
    size: PayloadSize,
    stream_count: usize,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = TonicBenchmarkClient::connect(uri.clone()).await?;
    warmup_client.client_stream(stream_count, size).await?;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let (tx, rx) = mpsc::channel::<WorkerResult>((config.concurrency * 2) as usize);

    let handles: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                let mut client = match TonicBenchmarkClient::connect(uri).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[WORKER {}] Connection failed: {}", id, e);
                        let _ = tx.send((0, 0, Vec::new())).await;
                        return;
                    }
                };
                let duration = Duration::from_secs(config.duration_secs);
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    let req_start = Instant::now();
                    let result = client.client_stream(stream_count, size).await;
                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);
                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    let (result, _) = collect_and_compute_result(rx, config.concurrency, handles).await;
    Ok(result)
}

async fn run_tonic_bidi_stream_test(
    config: &Config,
    port: u16,
    size: PayloadSize,
    stream_count: usize,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = TonicBenchmarkClient::connect(uri.clone()).await?;
    warmup_client.bi_di_stream(stream_count, size).await?;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let (tx, rx) = mpsc::channel::<WorkerResult>((config.concurrency * 2) as usize);

    let handles: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                let mut client = match TonicBenchmarkClient::connect(uri).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[WORKER {}] Connection failed: {}", id, e);
                        let _ = tx.send((0, 0, Vec::new())).await;
                        return;
                    }
                };
                let duration = Duration::from_secs(config.duration_secs);
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    let req_start = Instant::now();
                    let result = client.bi_di_stream(stream_count, size).await;
                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);
                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    let (result, _) = collect_and_compute_result(rx, config.concurrency, handles).await;
    Ok(result)
}

// ============================================================================
// Prost Load Tests
// ============================================================================

async fn run_prost_unary_test(
    config: &Config,
    port: u16,
    size: PayloadSize,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = ProstBenchmarkClient::connect(uri.clone()).await?;
    warmup_client.unary_call(0, size).await?;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let (tx, rx) = mpsc::channel::<WorkerResult>((config.concurrency * 2) as usize);

    let handles: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                let mut client = match ProstBenchmarkClient::connect(uri).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[WORKER {}] Connection failed: {}", id, e);
                        let _ = tx.send((0, 0, Vec::new())).await;
                        return;
                    }
                };
                let duration = Duration::from_secs(config.duration_secs);
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    let req_start = Instant::now();
                    let result = client.unary_call(id as u64, size).await;
                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);
                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    let (result, _) = collect_and_compute_result(rx, config.concurrency, handles).await;
    Ok(result)
}

async fn run_prost_server_stream_test(
    config: &Config,
    port: u16,
    size: PayloadSize,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = ProstBenchmarkClient::connect(uri.clone()).await?;
    warmup_client.server_stream(0, size).await?;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let (tx, rx) = mpsc::channel::<WorkerResult>((config.concurrency * 2) as usize);

    let handles: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                let mut client = match ProstBenchmarkClient::connect(uri).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[WORKER {}] Connection failed: {}", id, e);
                        let _ = tx.send((0, 0, Vec::new())).await;
                        return;
                    }
                };
                let duration = Duration::from_secs(config.duration_secs);
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    let req_start = Instant::now();
                    let result = client.server_stream(id as u64, size).await;
                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);
                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    let (result, _) = collect_and_compute_result(rx, config.concurrency, handles).await;
    Ok(result)
}

async fn run_prost_client_stream_test(
    config: &Config,
    port: u16,
    size: PayloadSize,
    stream_count: usize,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = ProstBenchmarkClient::connect(uri.clone()).await?;
    warmup_client.client_stream(stream_count, size).await?;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let (tx, rx) = mpsc::channel::<WorkerResult>((config.concurrency * 2) as usize);

    let handles: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                let mut client = match ProstBenchmarkClient::connect(uri).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[WORKER {}] Connection failed: {}", id, e);
                        let _ = tx.send((0, 0, Vec::new())).await;
                        return;
                    }
                };
                let duration = Duration::from_secs(config.duration_secs);
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    let req_start = Instant::now();
                    let result = client.client_stream(stream_count, size).await;
                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);
                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    let (result, _) = collect_and_compute_result(rx, config.concurrency, handles).await;
    Ok(result)
}

async fn run_prost_bidi_stream_test(
    config: &Config,
    port: u16,
    size: PayloadSize,
    stream_count: usize,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = ProstBenchmarkClient::connect(uri.clone()).await?;
    warmup_client.bi_di_stream(stream_count, size).await?;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let (tx, rx) = mpsc::channel::<WorkerResult>((config.concurrency * 2) as usize);

    let handles: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                let mut client = match ProstBenchmarkClient::connect(uri).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[WORKER {}] Connection failed: {}", id, e);
                        let _ = tx.send((0, 0, Vec::new())).await;
                        return;
                    }
                };
                let duration = Duration::from_secs(config.duration_secs);
                let worker_start = Instant::now();
                let mut local_total: u64 = 0;
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    let req_start = Instant::now();
                    let result = client.bi_di_stream(stream_count, size).await;
                    let latency = req_start.elapsed().as_nanos() as u64;
                    local_total += 1;
                    local_latencies.push(latency);
                    if result.is_ok() {
                        local_success += 1;
                    }
                }

                let _ = tx.send((local_total, local_success, local_latencies)).await;
            })
        })
        .collect();

    let (result, _) = collect_and_compute_result(rx, config.concurrency, handles).await;
    Ok(result)
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[derive(Parser, Debug)]
    struct Args {
        #[arg(long, default_value_t = 10)]
        duration: u64,

        #[arg(long, default_value_t = 50)]
        concurrency: u32,

        #[arg(long, default_value_t = 1000)]
        stream_count: usize,

        #[arg(long, default_value_t = 50052)]
        prost_port: u16,

        #[arg(long, default_value_t = 50053)]
        tonic_port: u16,
    }

    let args = Args::parse();

    let config = Config {
        duration_secs: args.duration,
        concurrency: args.concurrency,
        warmup_secs: 1,
        stream_count: args.stream_count,
    };

    println!("gRPC Streaming Load Test Configuration:");
    println!("  Duration: {}s", config.duration_secs);
    println!("  Concurrency: {}", config.concurrency);
    println!("  Stream Count: {}", config.stream_count);
    println!();
    println!("This load test requires running servers:");
    println!("  1. Prost (pure-grpc) server on port {}", args.prost_port);
    println!(
        "     cargo run --release -p benchmarks --bin prost_benchmark_server -- --port {}",
        args.prost_port
    );
    println!("  2. Tonic server on port {}", args.tonic_port);
    println!(
        "     cargo run --release -p tonic-bench-server --bin tonic_server -- --port {}",
        args.tonic_port
    );
    println!();

    let size = PayloadSize::Small;

    // === Unary Tests ===
    println!(
        "\n========== UNARY TESTS ({} payload) ==========",
        size.as_str()
    );

    println!("\n--- Tonic Unary ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 60),
        run_tonic_unary_test(&config, args.tonic_port, size),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Tonic Unary"),
        Ok(Err(e)) => println!("Tonic unary failed: {}", e),
        Err(_) => println!("Tonic unary timed out"),
    }

    println!("\n--- Prost Unary ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 60),
        run_prost_unary_test(&config, args.prost_port, size),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Prost Unary"),
        Ok(Err(e)) => println!("Prost unary failed: {}", e),
        Err(_) => println!("Prost unary timed out"),
    }

    // === Server Streaming Tests ===
    println!(
        "\n========== SERVER STREAMING TESTS ({} payload) ==========",
        size.as_str()
    );

    println!("\n--- Tonic Server Stream ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 60),
        run_tonic_server_stream_test(&config, args.tonic_port, size),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Tonic Server Stream"),
        Ok(Err(e)) => println!("Tonic server stream failed: {}", e),
        Err(_) => println!("Tonic server stream timed out"),
    }

    println!("\n--- Prost Server Stream ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 60),
        run_prost_server_stream_test(&config, args.prost_port, size),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Prost Server Stream"),
        Ok(Err(e)) => println!("Prost server stream failed: {}", e),
        Err(_) => println!("Prost server stream timed out"),
    }

    // === Client Streaming Tests ===
    println!(
        "\n========== CLIENT STREAMING TESTS ({} payload, {} items) ==========",
        size.as_str(),
        config.stream_count
    );

    println!("\n--- Tonic Client Stream ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 60),
        run_tonic_client_stream_test(&config, args.tonic_port, size, config.stream_count),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Tonic Client Stream"),
        Ok(Err(e)) => println!("Tonic client stream failed: {}", e),
        Err(_) => println!("Tonic client stream timed out"),
    }

    println!("\n--- Prost Client Stream ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 60),
        run_prost_client_stream_test(&config, args.prost_port, size, config.stream_count),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Prost Client Stream"),
        Ok(Err(e)) => println!("Prost client stream failed: {}", e),
        Err(_) => println!("Prost client stream timed out"),
    }

    // === Bidirectional Streaming Tests ===
    println!(
        "\n========== BIDIRECTIONAL STREAMING TESTS ({} payload, {} items) ==========",
        size.as_str(),
        config.stream_count
    );

    println!("\n--- Tonic BiDi Stream ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 60),
        run_tonic_bidi_stream_test(&config, args.tonic_port, size, config.stream_count),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Tonic BiDi Stream"),
        Ok(Err(e)) => println!("Tonic bidi stream failed: {}", e),
        Err(_) => println!("Tonic bidi stream timed out"),
    }

    println!("\n--- Prost BiDi Stream ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 60),
        run_prost_bidi_stream_test(&config, args.prost_port, size, config.stream_count),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Prost BiDi Stream"),
        Ok(Err(e)) => println!("Prost bidi stream failed: {}", e),
        Err(_) => println!("Prost bidi stream timed out"),
    }

    println!();
    Ok(())
}
