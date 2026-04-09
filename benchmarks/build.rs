fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_path = std::path::PathBuf::from("proto");
    let fbs_path = std::path::PathBuf::from("fbs");

    // Compile benchmark schemas (for load_test)
    grpc_build::compile_protos(&["proto/benchmark.proto"], &[&proto_path])?;
    grpc_build::compile_fbs(&["fbs/benchmark.fbs"], &[&fbs_path])?;

    // Compile todo schemas for pure-grpc servers and clients
    // Note: load test uses manual prost types for tonic client to avoid conflict
    grpc_build::compile_protos(&["proto/todo.proto"], &[&proto_path])?;
    grpc_build::compile_fbs(&["fbs/todo.fbs"], &[&fbs_path])?;

    Ok(())
}
