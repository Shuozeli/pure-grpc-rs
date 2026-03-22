use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

#[cfg(feature = "prost-codec")]
mod prost_bench {
    use super::*;
    use prost::Message;

    #[derive(Clone, PartialEq, Message)]
    struct BenchMessage {
        #[prost(string, tag = "1")]
        name: String,
        #[prost(int32, tag = "2")]
        id: i32,
        #[prost(bytes = "vec", tag = "3")]
        payload: Vec<u8>,
    }

    fn make_small_message() -> BenchMessage {
        BenchMessage {
            name: "hello".into(),
            id: 42,
            payload: vec![0u8; 64],
        }
    }

    fn make_large_message() -> BenchMessage {
        BenchMessage {
            name: "large-payload-benchmark-message".into(),
            id: 99999,
            payload: vec![0xAB; 65536],
        }
    }

    pub fn bench_encode(c: &mut Criterion) {
        let mut group = c.benchmark_group("prost_encode");

        let small = make_small_message();
        group.throughput(Throughput::Bytes(small.encoded_len() as u64));
        group.bench_function("small_77B", |b| {
            b.iter(|| {
                let _ = black_box(small.clone()).encode_to_vec();
            });
        });

        let large = make_large_message();
        group.throughput(Throughput::Bytes(large.encoded_len() as u64));
        group.bench_function("large_64KB", |b| {
            b.iter(|| {
                let _ = black_box(large.clone()).encode_to_vec();
            });
        });

        group.finish();
    }

    pub fn bench_decode(c: &mut Criterion) {
        let mut group = c.benchmark_group("prost_decode");

        let small = make_small_message();
        let small_bytes = small.encode_to_vec();
        group.throughput(Throughput::Bytes(small_bytes.len() as u64));
        group.bench_function("small_77B", |b| {
            b.iter(|| {
                let _ = BenchMessage::decode(black_box(small_bytes.as_slice())).unwrap();
            });
        });

        let large = make_large_message();
        let large_bytes = large.encode_to_vec();
        group.throughput(Throughput::Bytes(large_bytes.len() as u64));
        group.bench_function("large_64KB", |b| {
            b.iter(|| {
                let _ = BenchMessage::decode(black_box(large_bytes.as_slice())).unwrap();
            });
        });

        group.finish();
    }
}

#[cfg(feature = "gzip")]
mod compression_bench {
    use super::*;
    use bytes::BytesMut;
    use grpc_core::codec::compression::{compress, decompress, CompressionEncoding};

    pub fn bench_compress(c: &mut Criterion) {
        let mut group = c.benchmark_group("gzip_compress");

        let data = vec![0xABu8; 65536];
        group.throughput(Throughput::Bytes(data.len() as u64));
        group.bench_function("64KB", |b| {
            b.iter(|| {
                let mut dst = BytesMut::with_capacity(70000);
                compress(CompressionEncoding::Gzip, black_box(&data), &mut dst).unwrap();
            });
        });

        group.finish();
    }

    pub fn bench_decompress(c: &mut Criterion) {
        let mut group = c.benchmark_group("gzip_decompress");

        let data = vec![0xABu8; 65536];
        let compressed = {
            let mut dst = BytesMut::new();
            compress(CompressionEncoding::Gzip, &data, &mut dst).unwrap();
            dst.freeze()
        };

        group.throughput(Throughput::Bytes(data.len() as u64));
        group.bench_function("64KB", |b| {
            b.iter(|| {
                let mut dst = BytesMut::with_capacity(70000);
                decompress(CompressionEncoding::Gzip, black_box(&compressed), &mut dst).unwrap();
            });
        });

        group.finish();
    }
}

#[cfg(feature = "zstd")]
mod zstd_bench {
    use super::*;
    use bytes::BytesMut;
    use grpc_core::codec::compression::{compress, decompress, CompressionEncoding};

    pub fn bench_compress(c: &mut Criterion) {
        let mut group = c.benchmark_group("zstd_compress");

        let data = vec![0xABu8; 65536];
        group.throughput(Throughput::Bytes(data.len() as u64));
        group.bench_function("64KB", |b| {
            b.iter(|| {
                let mut dst = BytesMut::with_capacity(70000);
                compress(CompressionEncoding::Zstd, black_box(&data), &mut dst).unwrap();
            });
        });

        group.finish();
    }

    pub fn bench_decompress(c: &mut Criterion) {
        let mut group = c.benchmark_group("zstd_decompress");

        let data = vec![0xABu8; 65536];
        let compressed = {
            let mut dst = BytesMut::new();
            compress(CompressionEncoding::Zstd, &data, &mut dst).unwrap();
            dst.freeze()
        };

        group.throughput(Throughput::Bytes(data.len() as u64));
        group.bench_function("64KB", |b| {
            b.iter(|| {
                let mut dst = BytesMut::with_capacity(70000);
                decompress(CompressionEncoding::Zstd, black_box(&compressed), &mut dst).unwrap();
            });
        });

        group.finish();
    }
}

// Criterion group/main configuration based on enabled features
#[cfg(all(feature = "prost-codec", feature = "gzip", feature = "zstd"))]
criterion_group!(
    benches,
    prost_bench::bench_encode,
    prost_bench::bench_decode,
    compression_bench::bench_compress,
    compression_bench::bench_decompress,
    zstd_bench::bench_compress,
    zstd_bench::bench_decompress,
);

#[cfg(all(feature = "prost-codec", feature = "gzip", not(feature = "zstd")))]
criterion_group!(
    benches,
    prost_bench::bench_encode,
    prost_bench::bench_decode,
    compression_bench::bench_compress,
    compression_bench::bench_decompress,
);

#[cfg(all(feature = "prost-codec", not(feature = "gzip"), not(feature = "zstd")))]
criterion_group!(
    benches,
    prost_bench::bench_encode,
    prost_bench::bench_decode,
);

criterion_main!(benches);
