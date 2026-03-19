use grpc_core::body::Body;
use grpc_core::BoxFuture;
use grpc_core::Status;
use http::{Request, Response};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower_service::Service;

/// Trait object for a gRPC service that can be stored in the router.
///
/// Any `tower::Service<Request<Body>, Response=Response<Body>, Error=Infallible>`
/// that is also `Clone + Send + Sync + 'static` can be used.
trait GrpcService: Send + Sync + 'static {
    fn call_box(&self, req: Request<Body>) -> BoxFuture<Result<Response<Body>, Infallible>>;
}

/// Wrapper to make a cloneable service into a trait object.
struct ServiceWrapper<S> {
    inner: S,
}

impl<S> GrpcService for ServiceWrapper<S>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    fn call_box(&self, req: Request<Body>) -> BoxFuture<Result<Response<Body>, Infallible>> {
        let mut svc = self.inner.clone();
        Box::pin(async move { svc.call(req).await })
    }
}

/// A simple HashMap-based gRPC service router.
///
/// Routes requests by matching the service name from the URI path
/// (`/{service_name}/{method_name}`).
#[derive(Clone)]
pub struct Router {
    routes: HashMap<String, Arc<dyn GrpcService>>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    /// Add a service to the router.
    ///
    /// The service is registered under `/{service_name}` where `service_name`
    /// comes from `NamedService::NAME`.
    pub fn add_service<S>(mut self, name: &str, svc: S) -> Self
    where
        S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
            + Clone
            + Send
            + Sync
            + 'static,
        S::Future: Send + 'static,
    {
        let key = format!("/{name}");
        self.routes
            .insert(key, Arc::new(ServiceWrapper { inner: svc }));
        self
    }

    fn route(&self, path: &str) -> Option<&Arc<dyn GrpcService>> {
        // Path format: /{service_name}/{method_name}
        // Find the service by matching the prefix up to the second slash.
        let path = path.strip_prefix('/').unwrap_or(path);
        if let Some(slash_pos) = path.find('/') {
            let service_key = format!("/{}", &path[..slash_pos]);
            self.routes.get(&service_key)
        } else {
            None
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl Service<Request<Body>> for Router {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<Result<Response<Body>, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let path = req.uri().path().to_string();

        if let Some(svc) = self.route(&path) {
            svc.call_box(req)
        } else {
            Box::pin(async {
                let status = Status::unimplemented("service not found");
                let (parts, ()) = status.into_http::<()>().into_parts();
                Ok(Response::from_parts(parts, Body::empty()))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    #[derive(Clone)]
    struct MockService {
        name: String,
    }

    impl Service<Request<Body>> for MockService {
        type Response = Response<Body>;
        type Error = Infallible;
        type Future = std::future::Ready<Result<Response<Body>, Infallible>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: Request<Body>) -> Self::Future {
            let mut resp = Response::new(Body::empty());
            resp.headers_mut().insert(
                "x-service",
                http::HeaderValue::from_str(&self.name).unwrap(),
            );
            std::future::ready(Ok(resp))
        }
    }

    #[tokio::test]
    async fn routes_to_correct_service() {
        let mut router = Router::new().add_service(
            "helloworld.Greeter",
            MockService {
                name: "greeter".to_string(),
            },
        );

        let req = Request::builder()
            .uri("/helloworld.Greeter/SayHello")
            .body(Body::empty())
            .unwrap();

        let resp = router.call(req).await.unwrap();
        assert_eq!(resp.headers().get("x-service").unwrap(), "greeter");
    }

    #[tokio::test]
    async fn unknown_service_returns_unimplemented() {
        let mut router = Router::new();

        let req = Request::builder()
            .uri("/unknown.Service/Method")
            .body(Body::empty())
            .unwrap();

        let resp = router.call(req).await.unwrap();
        let status = resp.headers().get("grpc-status").unwrap();
        assert_eq!(status, "12"); // Code::Unimplemented
    }

    #[test]
    fn route_parsing() {
        let router = Router::new().add_service(
            "pkg.Svc",
            MockService {
                name: "test".to_string(),
            },
        );

        assert!(router.route("/pkg.Svc/Method").is_some());
        assert!(router.route("/pkg.Svc/AnotherMethod").is_some());
        assert!(router.route("/other.Svc/Method").is_none());
        assert!(router.route("/noservice").is_none());
    }
}
