fn main() {
    grpc_build::compile_protos(&["proto/helloworld.proto"], &["proto"]).unwrap();
}
