use grpc_core::body::Body;
use grpc_core::Status;
use http::{Request, Response};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower_service::Service;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A gRPC interceptor function.
///
/// Interceptors can inspect and modify requests before they reach the
/// service handler. They can also reject requests by returning an error.
///
/// # Example
///
/// ```ignore
/// fn auth_interceptor(req: Request<()>) -> Result<Request<()>, Status> {
///     let token = req.metadata().get("authorization")
///         .ok_or_else(|| Status::unauthenticated("missing auth token"))?;
///     Ok(req)
/// }
///
/// let service = InterceptedService::new(greeter_server, auth_interceptor);
/// ```
pub trait Interceptor: Clone + Send + Sync + 'static {
    fn intercept(&self, request: grpc_core::Request<()>) -> Result<grpc_core::Request<()>, Status>;
}

/// Blanket implementation: any `Fn(Request<()>) -> Result<Request<()>, Status>` is an Interceptor.
impl<F> Interceptor for F
where
    F: Fn(grpc_core::Request<()>) -> Result<grpc_core::Request<()>, Status>
        + Clone
        + Send
        + Sync
        + 'static,
{
    fn intercept(&self, request: grpc_core::Request<()>) -> Result<grpc_core::Request<()>, Status> {
        (self)(request)
    }
}

/// A service wrapped with an interceptor.
///
/// The interceptor runs before each request. If it returns `Err(Status)`,
/// the request is rejected without reaching the inner service.
#[derive(Clone)]
pub struct InterceptedService<S, I> {
    inner: S,
    interceptor: I,
}

impl<S, I> InterceptedService<S, I> {
    pub fn new(inner: S, interceptor: I) -> Self {
        Self { inner, interceptor }
    }
}

impl<S, I> Service<Request<Body>> for InterceptedService<S, I>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    I: Interceptor,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<Result<Response<Body>, Infallible>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // Extract metadata from the request for the interceptor
        let (parts, body) = req.into_parts();
        let metadata = grpc_core::MetadataMap::from_headers(parts.headers.clone());
        let interceptor_req =
            grpc_core::Request::from_parts(metadata, parts.extensions.clone(), ());

        // Run interceptor
        match self.interceptor.intercept(interceptor_req) {
            Ok(intercepted) => {
                // Merge any modified metadata/extensions back
                let (new_metadata, new_extensions, _) = intercepted.into_parts();
                let mut headers = new_metadata.into_headers();
                // Preserve original non-metadata headers (content-type, te, etc.)
                for (key, value) in &parts.headers {
                    if !headers.contains_key(key) {
                        headers.insert(key.clone(), value.clone());
                    }
                }
                let mut new_parts = parts;
                new_parts.headers = headers;
                new_parts.extensions = new_extensions;

                let req = Request::from_parts(new_parts, body);
                let mut inner = self.inner.clone();
                Box::pin(async move { inner.call(req).await })
            }
            Err(status) => Box::pin(async move { Ok(status.into_http()) }),
        }
    }
}

impl<S, I> crate::NamedService for InterceptedService<S, I>
where
    S: crate::NamedService,
{
    const NAME: &'static str = S::NAME;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct MockService;

    impl Service<Request<Body>> for MockService {
        type Response = Response<Body>;
        type Error = Infallible;
        type Future = std::future::Ready<Result<Response<Body>, Infallible>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: Request<Body>) -> Self::Future {
            std::future::ready(Ok(Response::new(Body::empty())))
        }
    }

    #[tokio::test]
    async fn interceptor_passes_through() {
        let svc = MockService;
        let interceptor = |req: grpc_core::Request<()>| Ok(req);
        let mut intercepted = InterceptedService::new(svc, interceptor);

        let req = Request::builder()
            .uri("/test.Svc/Method")
            .body(Body::empty())
            .unwrap();

        let resp = intercepted.call(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn interceptor_rejects_request() {
        let svc = MockService;
        let interceptor = |_req: grpc_core::Request<()>| Err(Status::unauthenticated("no token"));
        let mut intercepted = InterceptedService::new(svc, interceptor);

        let req = Request::builder()
            .uri("/test.Svc/Method")
            .body(Body::empty())
            .unwrap();

        let resp = intercepted.call(req).await.unwrap();
        let status = resp.headers().get("grpc-status").unwrap();
        assert_eq!(status, "16"); // Unauthenticated
    }

    #[tokio::test]
    async fn interceptor_can_read_metadata() {
        let svc = MockService;
        let interceptor = |req: grpc_core::Request<()>| {
            if req.metadata().get("authorization").is_none() {
                return Err(Status::unauthenticated("missing auth"));
            }
            Ok(req)
        };
        let mut intercepted = InterceptedService::new(svc, interceptor);

        // Without auth header — rejected
        let req = Request::builder()
            .uri("/test.Svc/Method")
            .body(Body::empty())
            .unwrap();
        let resp = intercepted.call(req).await.unwrap();
        assert_eq!(resp.headers().get("grpc-status").unwrap(), "16");

        // With auth header — passes
        let req = Request::builder()
            .uri("/test.Svc/Method")
            .header("authorization", "Bearer token123")
            .body(Body::empty())
            .unwrap();
        let resp = intercepted.call(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}
