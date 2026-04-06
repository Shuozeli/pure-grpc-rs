//! gRPC Load Test Binary
//!
//! Compares pure-grpc (FlatBuffers) vs tonic (Protobuf) performance
//!
//! Run with: `cargo run --release -p benchmarks --bin load_test -- [options]`
//!
//! Options:
//!   --duration <secs>     Test duration in seconds (default: 10)
//!   --concurrency <n>     Number of concurrent clients (default: 100)
//!   --payload <bytes>     Payload size in bytes (default: 1024)
//!   --flatbuffers-port    Port for FlatBuffers server (default: 50051)
//!   --protobuf-port       Port for Protobuf server (default: 50052)

use std::sync::Arc;
use std::time::{Duration, Instant};

use http::uri::PathAndQuery;
use prost::Message;
use tokio::sync::{mpsc, Barrier};
use tokio::time::timeout;

use grpc_client::{Channel, Grpc};
use grpc_codec_flatbuffers::{FlatBufferGrpcMessage, FlatBuffersCodec};
use grpc_core::codec::prost_codec::ProstCodec;
use grpc_core::extensions::GrpcMethod;
use grpc_core::request::IntoRequest;
use grpc_core::{Response, Status};

// Include generated FlatBuffers code from build.rs
// The generated code has snake_case warnings we need to suppress
#[allow(clippy::all)]
#[allow(non_snake_case)]
#[allow(dead_code)]
mod generated_flatbuffers {
    include!(concat!(env!("OUT_DIR"), "/benchmark_generated.rs"));
}

use generated_flatbuffers::benchmark::{
    BenchmarkRequest as FbsRequest, BenchmarkRequestT, BenchmarkResponse as FbsResponse,
    BenchmarkResponseT,
};

// Implement FlatBufferGrpcMessage for generated types
impl FlatBufferGrpcMessage for BenchmarkRequestT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut fbb = flatbuffers::FlatBufferBuilder::new();
        let _ = self.pack(&mut fbb);
        fbb.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let root = flatbuffers::root::<FbsRequest>(data)
            .map_err(|e| format!("flatbuffers decode error: {e}"))?;
        Ok(root.unpack())
    }
}

impl FlatBufferGrpcMessage for BenchmarkResponseT {
    fn encode_flatbuffer(&self) -> Vec<u8> {
        let mut fbb = flatbuffers::FlatBufferBuilder::new();
        let _ = self.pack(&mut fbb);
        fbb.finished_data().to_vec()
    }

    fn decode_flatbuffer(data: &[u8]) -> Result<Self, String> {
        let root = flatbuffers::root::<FbsResponse>(data)
            .map_err(|e| format!("flatbuffers decode error: {e}"))?;
        Ok(root.unpack())
    }
}

// ============================================================================
// Protobuf Types (inline for prost)
// ============================================================================

#[derive(Clone, Message)]
pub struct ProtobufBenchmarkRequest {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(string, tag = "2")]
    pub payload: String,
    #[prost(uint64, repeated, tag = "3")]
    pub numbers: Vec<u64>,
}

#[derive(Clone, Message)]
pub struct ProtobufBenchmarkResponse {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(string, tag = "2")]
    pub payload: String,
    #[prost(uint64, repeated, tag = "3")]
    pub numbers: Vec<u64>,
}

// ============================================================================
// FlatBuffers Client
// ============================================================================

#[derive(Debug, Clone)]
pub struct FlatBuffersClient {
    grpc: Grpc<Channel>,
}

impl FlatBuffersClient {
    pub async fn connect(uri: http::Uri) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let channel = Channel::connect(uri.clone()).await?;
        let grpc = Grpc::with_origin(channel, uri);
        Ok(Self { grpc })
    }

    pub async fn unary_call(
        &mut self,
        request: BenchmarkRequestT,
    ) -> Result<Response<BenchmarkResponseT>, Status> {
        let codec = FlatBuffersCodec::<BenchmarkRequestT, BenchmarkResponseT>::default();
        let path = PathAndQuery::from_static("/benchmark.BenchmarkService/UnaryCall");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("benchmark.BenchmarkService", "UnaryCall"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        self.grpc.unary(req, path, codec).await
    }
}

// ============================================================================
// Protobuf Client (using pure-grpc with prost codec)
// ============================================================================

#[derive(Debug, Clone)]
pub struct ProtobufClient {
    grpc: Grpc<Channel>,
}

impl ProtobufClient {
    pub async fn connect(uri: http::Uri) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let channel = Channel::connect(uri.clone()).await?;
        let grpc = Grpc::with_origin(channel, uri);
        Ok(Self { grpc })
    }

    pub async fn unary_call(
        &mut self,
        request: ProtobufBenchmarkRequest,
    ) -> Result<Response<ProtobufBenchmarkResponse>, Status> {
        let codec = ProstCodec::<ProtobufBenchmarkRequest, ProtobufBenchmarkResponse>::default();
        let path = PathAndQuery::from_static("/benchmark.BenchmarkService/UnaryCall");

        let mut req = request.into_request();
        req.extensions_mut()
            .insert(GrpcMethod::new("benchmark.BenchmarkService", "UnaryCall"));

        self.grpc
            .ready()
            .await
            .map_err(|e| Status::unknown(format!("Service was not ready: {}", e)))?;
        self.grpc.unary(req, path, codec).await
    }
}

// ============================================================================
// Load Test Configuration
// ============================================================================

#[derive(Debug, Clone)]
pub struct Config {
    pub duration_secs: u64,
    pub concurrency: u32,
    pub payload_size: usize,
    pub warmup_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            duration_secs: 10,
            concurrency: 100,
            payload_size: 1024,
            warmup_secs: 1,
        }
    }
}

// ============================================================================
// Load Test Functions
// ============================================================================

fn create_flatbuffers_request(id: u64, payload_size: usize) -> BenchmarkRequestT {
    BenchmarkRequestT {
        id,
        payload: Some("x".repeat(payload_size)),
        numbers: Some((0..100).collect()),
    }
}

fn create_protobuf_request(id: u64, payload_size: usize) -> ProtobufBenchmarkRequest {
    ProtobufBenchmarkRequest {
        id,
        payload: "x".repeat(payload_size),
        numbers: (0..100).collect(),
    }
}

async fn run_flatbuffers_load_test(
    config: &Config,
    port: u16,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = FlatBuffersClient::connect(uri.clone()).await?;
    let warmup_req = create_flatbuffers_request(0, config.payload_size);
    let _ = warmup_client.unary_call(warmup_req).await;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let barrier = Arc::new(Barrier::new(config.concurrency as usize));
    let start = Instant::now();
    let mut success_requests: u64 = 0;
    let mut latencies: Vec<u64> = Vec::new();
    let (tx, mut rx) = mpsc::channel::<(u64, Vec<u64>)>((config.concurrency * 2) as usize);

    // Spawn workers - each creates its own connection
    let workers: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let barrier = barrier.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                // Each worker creates its own client
                let mut client = FlatBuffersClient::connect(uri).await.unwrap();
                let duration = Duration::from_secs(config.duration_secs);
                let payload_size = config.payload_size;

                barrier.wait().await;
                let worker_start = Instant::now();
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    let req = create_flatbuffers_request(id as u64, payload_size);
                    let req_start = Instant::now();

                    if client.unary_call(req).await.is_ok() {
                        local_success += 1;
                        local_latencies.push(req_start.elapsed().as_nanos() as u64);
                    }
                }

                let _ = tx.send((local_success, local_latencies)).await;
            })
        })
        .collect();

    // Collect results
    while let Some((success, worker_latencies)) = rx.recv().await {
        success_requests += success;
        latencies.extend(worker_latencies);
    }

    for handle in workers {
        let _ = handle.await;
    }

    let elapsed = start.elapsed().as_secs_f64();
    latencies.sort();

    let qps = if elapsed > 0.0 {
        success_requests as f64 / elapsed
    } else {
        0.0
    };

    Ok(LoadTestResult {
        total_requests: success_requests,
        success_requests,
        qps,
        p50_ns: percentile(&latencies, 0.50),
        p95_ns: percentile(&latencies, 0.95),
        p99_ns: percentile(&latencies, 0.99),
    })
}

async fn run_protobuf_load_test(
    config: &Config,
    port: u16,
) -> Result<LoadTestResult, Box<dyn std::error::Error + Send + Sync>> {
    let uri: http::Uri = format!("http://127.0.0.1:{}", port).parse()?;

    // Warmup
    let mut warmup_client = ProtobufClient::connect(uri.clone()).await?;
    let warmup_req = create_protobuf_request(0, config.payload_size);
    let _ = warmup_client.unary_call(warmup_req).await;
    drop(warmup_client);
    tokio::time::sleep(Duration::from_secs(config.warmup_secs)).await;

    // Run load test
    let barrier = Arc::new(Barrier::new(config.concurrency as usize));
    let start = Instant::now();
    let mut success_requests: u64 = 0;
    let mut latencies: Vec<u64> = Vec::new();
    let (tx, mut rx) = mpsc::channel::<(u64, Vec<u64>)>((config.concurrency * 2) as usize);

    // Spawn workers - each creates its own connection
    let workers: Vec<_> = (0..config.concurrency)
        .map(|id| {
            let tx = tx.clone();
            let barrier = barrier.clone();
            let config = config.clone();
            let uri = uri.clone();

            tokio::spawn(async move {
                // Each worker creates its own client
                let mut client = ProtobufClient::connect(uri).await.unwrap();
                let duration = Duration::from_secs(config.duration_secs);
                let payload_size = config.payload_size;

                barrier.wait().await;
                let worker_start = Instant::now();
                let mut local_success: u64 = 0;
                let mut local_latencies: Vec<u64> = Vec::new();

                while worker_start.elapsed() < duration {
                    let req = create_protobuf_request(id as u64, payload_size);
                    let req_start = Instant::now();

                    if client.unary_call(req).await.is_ok() {
                        local_success += 1;
                        local_latencies.push(req_start.elapsed().as_nanos() as u64);
                    }
                }

                let _ = tx.send((local_success, local_latencies)).await;
            })
        })
        .collect();

    // Collect results
    while let Some((success, worker_latencies)) = rx.recv().await {
        success_requests += success;
        latencies.extend(worker_latencies);
    }

    for handle in workers {
        let _ = handle.await;
    }

    let elapsed = start.elapsed().as_secs_f64();
    latencies.sort();

    let qps = if elapsed > 0.0 {
        success_requests as f64 / elapsed
    } else {
        0.0
    };

    Ok(LoadTestResult {
        total_requests: success_requests,
        success_requests,
        qps,
        p50_ns: percentile(&latencies, 0.50),
        p95_ns: percentile(&latencies, 0.95),
        p99_ns: percentile(&latencies, 0.99),
    })
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[derive(Debug)]
struct LoadTestResult {
    total_requests: u64,
    success_requests: u64,
    qps: f64,
    p50_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
}

impl LoadTestResult {
    fn success_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.success_requests as f64 / self.total_requests as f64 * 100.0
        }
    }

    fn print(&self, name: &str) {
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

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct Args {
        #[arg(long, default_value_t = 10)]
        duration: u64,

        #[arg(long, default_value_t = 100)]
        concurrency: u32,

        #[arg(long, default_value_t = 1024)]
        payload: usize,

        #[arg(long, default_value_t = 50051)]
        flatbuffers_port: u16,

        #[arg(long, default_value_t = 50052)]
        protobuf_port: u16,
    }

    let args = Args::parse();

    let config = Config {
        duration_secs: args.duration,
        concurrency: args.concurrency,
        payload_size: args.payload,
        warmup_secs: 1,
    };

    println!("gRPC Load Test Configuration:");
    println!("  Duration: {}s", config.duration_secs);
    println!("  Concurrency: {}", config.concurrency);
    println!("  Payload size: {} bytes", config.payload_size);
    println!();
    println!("This load test requires running both servers separately:");
    println!("  1. FlatBuffers server on port {}", args.flatbuffers_port);
    println!("  2. Protobuf server on port {}", args.protobuf_port);

    // Run FlatBuffers load test if server is available
    println!("\n--- FlatBuffers (pure-grpc) Load Test ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 5),
        run_flatbuffers_load_test(&config, args.flatbuffers_port),
    )
    .await
    {
        Ok(Ok(result)) => result.print("FlatBuffers (pure-grpc)"),
        Ok(Err(e)) => println!("FlatBuffers load test failed: {}", e),
        Err(_) => println!("FlatBuffers load test timed out (server not running?)"),
    }

    // Run Protobuf load test if server is available
    println!("\n--- Protobuf (pure-grpc + prost) Load Test ---");
    match timeout(
        Duration::from_secs(config.duration_secs + 5),
        run_protobuf_load_test(&config, args.protobuf_port),
    )
    .await
    {
        Ok(Ok(result)) => result.print("Protobuf (pure-grpc+prost)"),
        Ok(Err(e)) => println!("Protobuf load test failed: {}", e),
        Err(_) => println!("Protobuf load test timed out (server not running?)"),
    }

    println!("\nNote: For a fair comparison with tonic, run this against tonic servers");

    Ok(())
}
