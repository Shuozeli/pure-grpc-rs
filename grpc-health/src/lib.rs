//! gRPC Health Checking service implementation.
//!
//! Implements `grpc.health.v1.Health` per the
//! [gRPC Health Checking Protocol](https://github.com/grpc/grpc/blob/master/doc/health-checking.md).
//!
//! Requires the `prost-codec` feature (enabled by default).
//!
//! # Usage
//!
//! ```ignore
//! use grpc_health::{HealthServer, health_service};
//!
//! let (health_service, health_handle) = health_service();
//! health_handle.set_serving("my.service.Name").await;
//!
//! let router = Router::new()
//!     .add_service("grpc.health.v1.Health", health_service);
//! ```

#[cfg(not(feature = "prost-codec"))]
compile_error!(
    "grpc-health requires the `prost-codec` feature. \
     Enable it with: grpc-health = { features = [\"prost-codec\"] }"
);

use grpc_core::body::Body;
#[cfg(feature = "prost-codec")]
use grpc_core::codec::prost_codec::ProstCodec;
use grpc_core::{Request, Response, Status};
use grpc_server::{Grpc, NamedService, ServerStreamingService, UnaryService};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::{watch, RwLock};

#[derive(Clone, PartialEq, prost::Message)]
pub struct HealthCheckRequest {
    #[prost(string, tag = "1")]
    pub service: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct HealthCheckResponse {
    #[prost(enumeration = "ServingStatus", tag = "1")]
    pub status: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, prost::Enumeration)]
#[repr(i32)]
pub enum ServingStatus {
    Unknown = 0,
    Serving = 1,
    NotServing = 2,
    ServiceUnknown = 3,
}

/// Shared state between HealthServer and HealthHandle.
struct HealthState {
    /// Current statuses (for Check RPC).
    statuses: RwLock<HashMap<String, ServingStatus>>,
    /// Watch senders (for Watch RPC). Created lazily per service name.
    watchers: RwLock<HashMap<String, watch::Sender<ServingStatus>>>,
}

type SharedState = Arc<HealthState>;

/// Handle for updating health status from application code.
#[derive(Clone)]
pub struct HealthHandle {
    state: SharedState,
}

impl HealthHandle {
    /// Set the serving status for a service.
    pub async fn set_status(&self, service: impl Into<String>, status: ServingStatus) {
        let service = service.into();
        self.state
            .statuses
            .write()
            .await
            .insert(service.clone(), status);

        // Notify any watchers
        let watchers = self.state.watchers.read().await;
        if let Some(tx) = watchers.get(&service) {
            if tx.send(status).is_err() {
                tracing::trace!(service = %service, "health watch: all receivers dropped");
            }
        }
    }

    /// Mark a service as serving.
    pub async fn set_serving(&self, service: impl Into<String>) {
        self.set_status(service, ServingStatus::Serving).await;
    }

    /// Mark a service as not serving.
    pub async fn set_not_serving(&self, service: impl Into<String>) {
        self.set_status(service, ServingStatus::NotServing).await;
    }
}

/// Create a health checking service and its control handle.
///
/// Returns `(HealthServer, HealthHandle)`. Use the server with a Router,
/// and the handle to update service statuses.
pub fn health_service() -> (HealthServer, HealthHandle) {
    let mut initial = HashMap::new();
    initial.insert(String::new(), ServingStatus::Serving);

    let state: SharedState = Arc::new(HealthState {
        statuses: RwLock::new(initial),
        watchers: RwLock::new(HashMap::new()),
    });

    let handle = HealthHandle {
        state: state.clone(),
    };
    let server = HealthServer { state };

    (server, handle)
}

/// The gRPC health checking server.
#[derive(Clone)]
pub struct HealthServer {
    state: SharedState,
}

impl NamedService for HealthServer {
    const NAME: &'static str = "grpc.health.v1.Health";
}

use grpc_core::{BoxFuture, BoxStream};

struct CheckSvc {
    state: SharedState,
}

impl UnaryService<HealthCheckRequest> for CheckSvc {
    type Response = HealthCheckResponse;
    type Future = BoxFuture<Result<Response<HealthCheckResponse>, Status>>;

    fn call(&mut self, request: Request<HealthCheckRequest>) -> Self::Future {
        let state = self.state.clone();
        let service_name = request.into_inner().service;

        Box::pin(async move {
            let map = state.statuses.read().await;
            let status = match map.get(&service_name) {
                Some(&s) => s,
                None => ServingStatus::ServiceUnknown,
            };

            Ok(Response::new(HealthCheckResponse {
                status: status as i32,
            }))
        })
    }
}

struct WatchSvc {
    state: SharedState,
}

impl ServerStreamingService<HealthCheckRequest> for WatchSvc {
    type Response = HealthCheckResponse;
    type ResponseStream = BoxStream<Result<HealthCheckResponse, Status>>;
    type Future = BoxFuture<Result<Response<Self::ResponseStream>, Status>>;

    fn call(&mut self, request: Request<HealthCheckRequest>) -> Self::Future {
        let state = self.state.clone();
        let service_name = request.into_inner().service;

        Box::pin(async move {
            // Get or create a watch channel for this service
            let initial_status = {
                let statuses = state.statuses.read().await;
                statuses.get(&service_name).copied()
            };

            let rx = {
                let mut watchers = state.watchers.write().await;
                if let Some(tx) = watchers.get(&service_name) {
                    tx.subscribe()
                } else {
                    let initial = initial_status.unwrap_or(ServingStatus::ServiceUnknown);
                    let (tx, rx) = watch::channel(initial);
                    watchers.insert(service_name, tx);
                    rx
                }
            };

            let stream = tokio_stream::wrappers::WatchStream::new(rx).map(|status| {
                Ok(HealthCheckResponse {
                    status: status as i32,
                })
            });

            Ok(Response::new(Box::pin(stream) as BoxStream<_>))
        })
    }
}

use tokio_stream::StreamExt as _;

impl tower_service::Service<http::Request<Body>> for HealthServer {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<Result<http::Response<Body>, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<Body>) -> Self::Future {
        let state = self.state.clone();

        match req.uri().path() {
            "/grpc.health.v1.Health/Check" => Box::pin(async move {
                let mut grpc =
                    Grpc::new(ProstCodec::<HealthCheckResponse, HealthCheckRequest>::default());
                Ok(grpc.unary(CheckSvc { state }, req).await)
            }),
            "/grpc.health.v1.Health/Watch" => Box::pin(async move {
                let mut grpc =
                    Grpc::new(ProstCodec::<HealthCheckResponse, HealthCheckRequest>::default());
                Ok(grpc.server_streaming(WatchSvc { state }, req).await)
            }),
            _ => Box::pin(async {
                let status = Status::unimplemented("");
                Ok(status.into_http())
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_handle_set_serving() {
        let (_, handle) = health_service();
        handle.set_serving("my.Service").await;

        let map = handle.state.statuses.read().await;
        assert_eq!(map.get("my.Service"), Some(&ServingStatus::Serving));
    }

    #[tokio::test]
    async fn health_handle_set_not_serving() {
        let (_, handle) = health_service();
        handle.set_not_serving("my.Service").await;

        let map = handle.state.statuses.read().await;
        assert_eq!(map.get("my.Service"), Some(&ServingStatus::NotServing));
    }

    #[tokio::test]
    async fn default_empty_service_is_serving() {
        let (_, handle) = health_service();
        let map = handle.state.statuses.read().await;
        assert_eq!(map.get(""), Some(&ServingStatus::Serving));
    }

    #[tokio::test]
    async fn check_unknown_service() {
        let (server, _) = health_service();
        let mut svc = CheckSvc {
            state: server.state.clone(),
        };
        let req = Request::new(HealthCheckRequest {
            service: "unknown.Service".into(),
        });
        let resp = svc.call(req).await.unwrap();
        assert_eq!(resp.get_ref().status, ServingStatus::ServiceUnknown as i32);
    }

    #[tokio::test]
    async fn check_serving_service() {
        let (server, handle) = health_service();
        handle.set_serving("my.Svc").await;

        let mut svc = CheckSvc {
            state: server.state.clone(),
        };
        let req = Request::new(HealthCheckRequest {
            service: "my.Svc".into(),
        });
        let resp = svc.call(req).await.unwrap();
        assert_eq!(resp.get_ref().status, ServingStatus::Serving as i32);
    }

    #[tokio::test]
    async fn watch_initial_status() {
        let (server, handle) = health_service();
        handle.set_serving("my.Svc").await;

        let mut svc = WatchSvc {
            state: server.state.clone(),
        };
        let req = Request::new(HealthCheckRequest {
            service: "my.Svc".into(),
        });
        let resp = svc.call(req).await.unwrap();
        let mut stream = resp.into_inner();

        // First message should be current status
        use tokio_stream::StreamExt;
        let msg = stream.next().await.unwrap().unwrap();
        assert_eq!(msg.status, ServingStatus::Serving as i32);
    }

    #[tokio::test]
    async fn watch_status_change() {
        let (server, handle) = health_service();
        handle.set_serving("my.Svc").await;

        let mut svc = WatchSvc {
            state: server.state.clone(),
        };
        let req = Request::new(HealthCheckRequest {
            service: "my.Svc".into(),
        });
        let resp = svc.call(req).await.unwrap();
        let mut stream = resp.into_inner();

        use tokio_stream::StreamExt;
        // Consume initial value
        let _ = stream.next().await.unwrap().unwrap();

        // Change status
        handle.set_not_serving("my.Svc").await;

        let msg = stream.next().await.unwrap().unwrap();
        assert_eq!(msg.status, ServingStatus::NotServing as i32);
    }

    #[tokio::test]
    async fn watch_unknown_then_registered() {
        let (server, handle) = health_service();

        let mut svc = WatchSvc {
            state: server.state.clone(),
        };
        let req = Request::new(HealthCheckRequest {
            service: "new.Svc".into(),
        });
        let resp = svc.call(req).await.unwrap();
        let mut stream = resp.into_inner();

        use tokio_stream::StreamExt;
        // Initially unknown
        let msg = stream.next().await.unwrap().unwrap();
        assert_eq!(msg.status, ServingStatus::ServiceUnknown as i32);

        // Register as serving
        handle.set_serving("new.Svc").await;

        let msg = stream.next().await.unwrap().unwrap();
        assert_eq!(msg.status, ServingStatus::Serving as i32);
    }
}
