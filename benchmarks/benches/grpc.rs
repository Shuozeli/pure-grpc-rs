//! gRPC Benchmark: pure-grpc-rs (FlatBuffers) vs tonic (Protobuf)
//!
//! This benchmark measures encoding/decoding performance, which is the main
//! difference between FlatBuffers and Protobuf.
//!
//! Run with: `cargo bench -p benchmarks`

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::time::Duration;

// ============================================================================
// Protobuf Benchmark Types
// ============================================================================

#[derive(Clone, prost::Message)]
pub struct BenchmarkRequest {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(string, tag = "2")]
    pub payload: String,
    #[prost(uint64, repeated, tag = "3")]
    pub numbers: Vec<u64>,
}

#[derive(Clone, prost::Message)]
pub struct BenchmarkResponse {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(string, tag = "2")]
    pub payload: String,
    #[prost(uint64, repeated, tag = "3")]
    pub numbers: Vec<u64>,
}

pub mod proto {
    use super::*;

    pub fn encode_request(request: &BenchmarkRequest) -> Vec<u8> {
        let mut buf = Vec::new();
        prost::Message::encode(request, &mut buf).unwrap();
        buf
    }

    pub fn decode_request(buf: &[u8]) -> BenchmarkRequest {
        prost::Message::decode(buf).unwrap()
    }

    pub fn encode_response(response: &BenchmarkResponse) -> Vec<u8> {
        let mut buf = Vec::new();
        prost::Message::encode(response, &mut buf).unwrap();
        buf
    }

    pub fn decode_response(buf: &[u8]) -> BenchmarkResponse {
        prost::Message::decode(buf).unwrap()
    }
}

// ============================================================================
// FlatBuffers Benchmark Types
// ============================================================================

// Include generated FlatBuffers code from build.rs
include!(concat!(env!("OUT_DIR"), "/benchmark_generated.rs"));

use crate::benchmark::{
    BenchmarkRequest as FbsRequest, BenchmarkRequestT, BenchmarkResponse as FbsResponse,
    BenchmarkResponseT,
};
use grpc_codec_flatbuffers::FlatBufferGrpcMessage;

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
// Benchmark Data
// ============================================================================

fn create_proto_request(id: u64) -> BenchmarkRequest {
    BenchmarkRequest {
        id,
        payload: "x".repeat(1024), // 1KB payload
        numbers: (0..100).collect(),
    }
}

fn create_proto_response(id: u64) -> BenchmarkResponse {
    BenchmarkResponse {
        id,
        payload: "x".repeat(1024), // 1KB payload
        numbers: (0..100).collect(),
    }
}

fn create_flatbuffers_request(id: u64) -> BenchmarkRequestT {
    BenchmarkRequestT {
        id,
        payload: Some("x".repeat(1024)),
        numbers: Some((0..100).collect()),
    }
}

fn create_flatbuffers_response(id: u64) -> BenchmarkResponseT {
    BenchmarkResponseT {
        id,
        payload: Some("x".repeat(1024)),
        numbers: Some((0..100).collect()),
    }
}

// ============================================================================
// Benchmarks - Protobuf
// ============================================================================

fn protobuf_encode_request(c: &mut Criterion) {
    let request = create_proto_request(1);

    c.benchmark_group("protobuf/encode/request")
        .bench_function("encode", |b| {
            b.iter(|| {
                let encoded = proto::encode_request(black_box(&request));
                black_box(encoded)
            });
        });
}

fn protobuf_decode_request(c: &mut Criterion) {
    let encoded = proto::encode_request(&create_proto_request(1));

    c.benchmark_group("protobuf/decode/request")
        .bench_function("decode", |b| {
            b.iter(|| {
                let decoded = proto::decode_request(black_box(&encoded));
                black_box(decoded)
            });
        });
}

fn protobuf_encode_response(c: &mut Criterion) {
    let response = create_proto_response(1);

    c.benchmark_group("protobuf/encode/response")
        .bench_function("encode", |b| {
            b.iter(|| {
                let encoded = proto::encode_response(black_box(&response));
                black_box(encoded)
            });
        });
}

fn protobuf_decode_response(c: &mut Criterion) {
    let encoded = proto::encode_response(&create_proto_response(1));

    c.benchmark_group("protobuf/decode/response")
        .bench_function("decode", |b| {
            b.iter(|| {
                let decoded = proto::decode_response(black_box(&encoded));
                black_box(decoded)
            });
        });
}

fn protobuf_round_trip(c: &mut Criterion) {
    let request = create_proto_request(1);

    c.benchmark_group("protobuf/round_trip")
        .bench_function("encode+decode", |b| {
            b.iter(|| {
                let encoded = proto::encode_request(black_box(&request));
                let decoded = proto::decode_request(black_box(&encoded));
                black_box(decoded)
            });
        });
}

// ============================================================================
// Benchmarks - FlatBuffers
// ============================================================================

fn flatbuffers_encode_request(c: &mut Criterion) {
    let request = create_flatbuffers_request(1);

    c.benchmark_group("flatbuffers/encode/request")
        .bench_function("encode", |b| {
            b.iter(|| {
                let encoded = BenchmarkRequestT::encode_flatbuffer(black_box(&request));
                black_box(encoded)
            });
        });
}

fn flatbuffers_decode_request(c: &mut Criterion) {
    let request = create_flatbuffers_request(1);
    let encoded = BenchmarkRequestT::encode_flatbuffer(&request);

    c.benchmark_group("flatbuffers/decode/request")
        .bench_function("decode", |b| {
            b.iter(|| {
                let decoded = BenchmarkRequestT::decode_flatbuffer(black_box(&encoded));
                black_box(decoded)
            });
        });
}

fn flatbuffers_encode_response(c: &mut Criterion) {
    let response = create_flatbuffers_response(1);

    c.benchmark_group("flatbuffers/encode/response")
        .bench_function("encode", |b| {
            b.iter(|| {
                let encoded = BenchmarkResponseT::encode_flatbuffer(black_box(&response));
                black_box(encoded)
            });
        });
}

fn flatbuffers_decode_response(c: &mut Criterion) {
    let response = create_flatbuffers_response(1);
    let encoded = BenchmarkResponseT::encode_flatbuffer(&response);

    c.benchmark_group("flatbuffers/decode/response")
        .bench_function("decode", |b| {
            b.iter(|| {
                let decoded = BenchmarkResponseT::decode_flatbuffer(black_box(&encoded));
                black_box(decoded)
            });
        });
}

fn flatbuffers_round_trip(c: &mut Criterion) {
    let request = create_flatbuffers_request(1);

    c.benchmark_group("flatbuffers/round_trip")
        .bench_function("encode+decode", |b| {
            b.iter(|| {
                let encoded = BenchmarkRequestT::encode_flatbuffer(black_box(&request));
                let decoded = BenchmarkRequestT::decode_flatbuffer(black_box(&encoded));
                black_box(decoded)
            });
        });
}

// ============================================================================
// Run all benchmarks
// ============================================================================

criterion_group!(
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    targets =
        protobuf_encode_request,
        protobuf_decode_request,
        protobuf_encode_response,
        protobuf_decode_response,
        protobuf_round_trip,
        flatbuffers_encode_request,
        flatbuffers_decode_request,
        flatbuffers_encode_response,
        flatbuffers_decode_response,
        flatbuffers_round_trip,
);

criterion_main!(benches);
