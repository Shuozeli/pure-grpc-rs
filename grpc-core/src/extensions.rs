/// A gRPC Method info extension.
///
/// Inserted into `http::Extensions` by generated code so interceptors can
/// inspect which RPC is being called.
#[derive(Debug, Clone)]
pub struct GrpcMethod {
    service: &'static str,
    method: &'static str,
}

impl GrpcMethod {
    pub fn new(service: &'static str, method: &'static str) -> Self {
        Self { service, method }
    }

    pub fn service(&self) -> &str {
        self.service
    }

    pub fn method(&self) -> &str {
        self.method
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_method_accessors() {
        let m = GrpcMethod::new("helloworld.Greeter", "SayHello");
        assert_eq!(m.service(), "helloworld.Greeter");
        assert_eq!(m.method(), "SayHello");
    }
}
