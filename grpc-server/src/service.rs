use grpc_core::{Request, Response, Status, Streaming};
use std::future::Future;
use tokio_stream::Stream;

/// A trait to provide a static reference to the service's name,
/// used for routing within the router.
pub trait NamedService {
    /// The fully-qualified gRPC service name (e.g., `"helloworld.Greeter"`).
    const NAME: &'static str;
}

/// Handler for unary RPCs: single request, single response.
pub trait UnaryService<R> {
    type Response;
    type Future: Future<Output = Result<Response<Self::Response>, Status>>;

    fn call(&mut self, request: Request<R>) -> Self::Future;
}

/// Handler for server-streaming RPCs: single request, stream of responses.
pub trait ServerStreamingService<R> {
    type Response;
    type ResponseStream: Stream<Item = Result<Self::Response, Status>>;
    type Future: Future<Output = Result<Response<Self::ResponseStream>, Status>>;

    fn call(&mut self, request: Request<R>) -> Self::Future;
}

/// Handler for client-streaming RPCs: stream of requests, single response.
pub trait ClientStreamingService<R> {
    type Response;
    type Future: Future<Output = Result<Response<Self::Response>, Status>>;

    fn call(&mut self, request: Request<Streaming<R>>) -> Self::Future;
}

/// Handler for bidirectional-streaming RPCs: stream of requests, stream of responses.
pub trait StreamingService<R> {
    type Response;
    type ResponseStream: Stream<Item = Result<Self::Response, Status>>;
    type Future: Future<Output = Result<Response<Self::ResponseStream>, Status>>;

    fn call(&mut self, request: Request<Streaming<R>>) -> Self::Future;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verify the traits compile with a concrete type
    struct Echo;

    impl UnaryService<String> for Echo {
        type Response = String;
        type Future = std::future::Ready<Result<Response<String>, Status>>;

        fn call(&mut self, request: Request<String>) -> Self::Future {
            let msg = request.into_inner();
            std::future::ready(Ok(Response::new(msg)))
        }
    }

    #[test]
    fn unary_service_compiles() {
        let mut svc = Echo;
        let req = Request::new("hello".to_string());
        let _fut = svc.call(req);
    }
}
