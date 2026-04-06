fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fbs_path = std::path::PathBuf::from("fbs");
    let proto_path = std::path::PathBuf::from("proto");

    // Generate Rust types from FlatBuffers schema
    grpc_build::compile_fbs(&["fbs/benchmark.fbs"], &[&fbs_path])?;

    // Generate Rust types + gRPC stubs from Protobuf schema
    grpc_build::compile_protos(&["proto/benchmark.proto"], &[&proto_path])?;

    Ok(())
}
