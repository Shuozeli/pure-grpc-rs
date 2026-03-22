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

// Macro for compression benchmarks — same pattern for each encoding
macro_rules! compression_bench_mod {
    ($mod_name:ident, $feature:literal, $encoding_variant:ident, $label:literal) => {
        #[cfg(feature = $feature)]
        mod $mod_name {
            use super::*;
            use bytes::BytesMut;
            use grpc_core::codec::compression::{compress, decompress, CompressionEncoding};

            pub fn bench_compress(c: &mut Criterion) {
                let mut group = c.benchmark_group(concat!($label, "_compress"));
                let data = vec![0xABu8; 65536];
                group.throughput(Throughput::Bytes(data.len() as u64));
                group.bench_function("64KB", |b| {
                    b.iter(|| {
                        let mut dst = BytesMut::with_capacity(70000);
                        compress(
                            CompressionEncoding::$encoding_variant,
                            black_box(&data),
                            &mut dst,
                        )
                        .unwrap();
                    });
                });
                group.finish();
            }

            pub fn bench_decompress(c: &mut Criterion) {
                let mut group = c.benchmark_group(concat!($label, "_decompress"));
                let data = vec![0xABu8; 65536];
                let compressed = {
                    let mut dst = BytesMut::new();
                    compress(CompressionEncoding::$encoding_variant, &data, &mut dst).unwrap();
                    dst.freeze()
                };
                group.throughput(Throughput::Bytes(data.len() as u64));
                group.bench_function("64KB", |b| {
                    b.iter(|| {
                        let mut dst = BytesMut::with_capacity(70000);
                        decompress(
                            CompressionEncoding::$encoding_variant,
                            black_box(&compressed),
                            &mut dst,
                        )
                        .unwrap();
                    });
                });
                group.finish();
            }
        }
    };
}

compression_bench_mod!(gzip_bench, "gzip", Gzip, "gzip");
compression_bench_mod!(deflate_bench, "deflate", Deflate, "deflate");
compression_bench_mod!(zstd_bench, "zstd", Zstd, "zstd");

// Use a single criterion_group that always includes prost benchmarks.
// Compression benchmarks are added via wrapper functions that are no-ops
// when the feature is disabled.
fn bench_all(c: &mut Criterion) {
    #[cfg(feature = "prost-codec")]
    {
        prost_bench::bench_encode(c);
        prost_bench::bench_decode(c);
    }
    #[cfg(feature = "gzip")]
    {
        gzip_bench::bench_compress(c);
        gzip_bench::bench_decompress(c);
    }
    #[cfg(feature = "deflate")]
    {
        deflate_bench::bench_compress(c);
        deflate_bench::bench_decompress(c);
    }
    #[cfg(feature = "zstd")]
    {
        zstd_bench::bench_compress(c);
        zstd_bench::bench_decompress(c);
    }
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
