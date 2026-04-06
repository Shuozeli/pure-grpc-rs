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
    fallback: Option<Arc<dyn GrpcService>>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
            fallback: None,
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

    /// Add a fallback service to handle requests that do not match any gRPC route.
    ///
    /// The fallback service will receive requests mapped to `Request<grpc_core::body::Body>`
    /// and can return responses with any type `B` that implements `http_body::Body`.
    pub fn fallback<S, B>(mut self, svc: S) -> Self
    where
        S: Service<Request<Body>, Response = Response<B>, Error = Infallible>
            + Clone
            + Send
            + Sync
            + 'static,
        S::Future: Send + 'static,
        B: http_body::Body<Data = bytes::Bytes> + Send + 'static,
        B::Error: Into<grpc_core::BoxError>,
    {
        self.fallback = Some(Arc::new(FallbackWrapper { inner: svc }));
        self
    }

    fn route(&self, path: &str) -> Option<&Arc<dyn GrpcService>> {
        // Path format: /{service_name}/{method_name}
        // Find the service by checking if path starts with "/{service_name}/"
        for (key, svc) in &self.routes {
            if path.starts_with(key.as_str()) && path[key.len()..].starts_with('/') {
                return Some(svc);
            }
        }
        None
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl Router {
    /// Apply a tower `Layer` to the router, returning a new `Service`.
    ///
    /// This enables composing standard tower middleware (rate limiting, tracing,
    /// concurrency limits, etc.) with gRPC services.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use tower::ServiceBuilder;
    /// use std::time::Duration;
    ///
    /// let router = Router::new()
    ///     .add_service("my.Service", my_service);
    ///
    /// let layered = router.into_layered(
    ///     ServiceBuilder::new()
    ///         .concurrency_limit(100)
    ///         .timeout(Duration::from_secs(30))
    /// );
    ///
    /// Server::builder().serve(addr, layered).await?;
    /// ```
    pub fn into_layered<L>(self, layer: L) -> L::Service
    where
        L: tower::Layer<Self>,
    {
        layer.layer(self)
    }
}

struct FallbackWrapper<S> {
    inner: S,
}

impl<S, B> GrpcService for FallbackWrapper<S>
where
    S: Service<Request<Body>, Response = Response<B>, Error = Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
    B: http_body::Body<Data = bytes::Bytes> + Send + 'static,
    B::Error: Into<grpc_core::BoxError>,
{
    fn call_box(&self, req: Request<Body>) -> BoxFuture<Result<Response<Body>, Infallible>> {
        let mut svc = self.inner.clone();
        Box::pin(async move {
            let resp = svc.call(req).await?;
            Ok(resp.map(Body::new))
        })
    }
}

impl<B> Service<Request<B>> for Router
where
    B: http_body::Body<Data = bytes::Bytes> + Send + 'static,
    B::Error: Into<grpc_core::BoxError>,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<Result<Response<Body>, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let path = req.uri().path().to_owned();
        let (parts, body) = req.into_parts();
        let req = Request::from_parts(parts, Body::new(body));

        if let Some(svc) = self.route(&path) {
            svc.call_box(req)
        } else if let Some(fallback) = &self.fallback {
            fallback.call_box(req)
        } else {
            Box::pin(async {
                let status = Status::unimplemented("service not found");
                Ok(status.into_http())
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

    // S14: Router::into_layered — apply a tower layer to the router
    #[tokio::test]
    async fn into_layered_applies_layer() {
        let router = Router::new().add_service(
            "helloworld.Greeter",
            MockService {
                name: "greeter".to_string(),
            },
        );

        // Apply identity layer — should produce a working service
        let mut layered = router.into_layered(tower::layer::util::Identity::new());

        let req = Request::builder()
            .uri("/helloworld.Greeter/SayHello")
            .body(http_body_util::Empty::<bytes::Bytes>::new())
            .unwrap();

        let resp = tower_service::Service::call(&mut layered, req)
            .await
            .unwrap();
        assert_eq!(resp.headers().get("x-service").unwrap(), "greeter");
    }

    #[tokio::test]
    async fn fallback_routes_to_fallback_service() {
        // A standard tower service that just returns a text response with a Full body.
        let fallback_svc = tower::service_fn(|_req| async {
            let resp = Response::builder()
                .header("x-fallback", "true")
                .body(http_body_util::Full::new(bytes::Bytes::from("hello")))
                .unwrap();
            Ok::<_, Infallible>(resp)
        });

        let mut router = Router::new()
            .add_service(
                "helloworld.Greeter",
                MockService {
                    name: "greeter".to_string(),
                },
            )
            .fallback(fallback_svc);

        // Request an unknown route
        let req = Request::builder()
            .uri("/static/index.html")
            .body(http_body_util::Empty::<bytes::Bytes>::new())
            .unwrap();

        let resp = tower_service::Service::call(&mut router, req)
            .await
            .unwrap();

        assert_eq!(resp.headers().get("x-fallback").unwrap(), "true");
    }
}
