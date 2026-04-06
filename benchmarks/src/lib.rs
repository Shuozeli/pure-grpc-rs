//! Benchmarks for pure-grpc-rs (FlatBuffers) vs tonic (Protobuf)
//!
//! This crate contains benchmark scenarios comparing:
//! - pure-grpc-rs with FlatBuffers codec
//! - tonic with Protobuf codec
//!
//! Run with: `cargo bench -p benchmarks`

use std::time::Duration;

/// Number of messages to stream in streaming benchmarks
pub const STREAM_COUNT: usize = 1000;

/// Size of payload in bytes
pub const PAYLOAD_SIZE: usize = 1024;

/// Common benchmark message data
pub struct BenchmarkData {
    pub id: u64,
    pub payload: Vec<u8>,
    pub numbers: Vec<u64>,
}

impl BenchmarkData {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            payload: vec![0u8; PAYLOAD_SIZE],
            numbers: (0..100).collect(),
        }
    }
}

/// Benchmark configuration
pub struct BenchmarkConfig {
    pub warmup_duration: Duration,
    pub measurement_duration: Duration,
    pub stream_count: usize,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            warmup_duration: Duration::from_secs(1),
            measurement_duration: Duration::from_secs(3),
            stream_count: STREAM_COUNT,
        }
    }
}
