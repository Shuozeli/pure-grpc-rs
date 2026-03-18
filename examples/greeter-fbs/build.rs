fn main() {
    grpc_build::compile_fbs(&["schema/greeter.fbs"], &["schema"]).unwrap();
}
