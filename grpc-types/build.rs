fn main() {
    grpc_build::compile_protos(&["proto/google_rpc_types.proto"], &["proto"]).unwrap();
}
