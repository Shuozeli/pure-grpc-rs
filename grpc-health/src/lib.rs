//! gRPC Health Checking service implementation.
//!
//! Implements `grpc.health.v1.Health` per the
//! [gRPC Health Checking Protocol](https://github.com/grpc/grpc/blob/master/doc/health-checking.md).
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

use grpc_core::body::Body;
use grpc_core::codec::prost_codec::ProstCodec;
use grpc_core::{Request, Response, Status};
use grpc_server::{Grpc, NamedService, UnaryService};
use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::RwLock;

// --- Protobuf messages (hand-written, matching grpc.health.v1) ---

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

// --- Health service state ---

type ServiceStatusMap = Arc<RwLock<HashMap<String, ServingStatus>>>;

/// Handle for updating health status from application code.
#[derive(Clone)]
pub struct HealthHandle {
    statuses: ServiceStatusMap,
}

impl HealthHandle {
    /// Set the serving status for a service.
    pub async fn set_status(&self, service: impl Into<String>, status: ServingStatus) {
        self.statuses.write().await.insert(service.into(), status);
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
    let statuses: ServiceStatusMap = Arc::new(RwLock::new(initial));

    let handle = HealthHandle {
        statuses: statuses.clone(),
    };

    let server = HealthServer { statuses };

    (server, handle)
}

// --- Server implementation ---

/// The gRPC health checking server.
#[derive(Clone)]
pub struct HealthServer {
    statuses: ServiceStatusMap,
}

impl NamedService for HealthServer {
    const NAME: &'static str = "grpc.health.v1.Health";
}

struct CheckSvc {
    statuses: ServiceStatusMap,
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

impl UnaryService<HealthCheckRequest> for CheckSvc {
    type Response = HealthCheckResponse;
    type Future = BoxFuture<Result<Response<HealthCheckResponse>, Status>>;

    fn call(&mut self, request: Request<HealthCheckRequest>) -> Self::Future {
        let statuses = self.statuses.clone();
        let service_name = request.into_inner().service;

        Box::pin(async move {
            let map = statuses.read().await;
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

impl tower_service::Service<http::Request<Body>> for HealthServer {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<Result<http::Response<Body>, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<Body>) -> Self::Future {
        let statuses = self.statuses.clone();

        match req.uri().path() {
            "/grpc.health.v1.Health/Check" => Box::pin(async move {
                let mut grpc =
                    Grpc::new(ProstCodec::<HealthCheckResponse, HealthCheckRequest>::default());
                Ok(grpc.unary(CheckSvc { statuses }, req).await)
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

        let map = handle.statuses.read().await;
        assert_eq!(map.get("my.Service"), Some(&ServingStatus::Serving));
    }

    #[tokio::test]
    async fn health_handle_set_not_serving() {
        let (_, handle) = health_service();
        handle.set_not_serving("my.Service").await;

        let map = handle.statuses.read().await;
        assert_eq!(map.get("my.Service"), Some(&ServingStatus::NotServing));
    }

    #[tokio::test]
    async fn default_empty_service_is_serving() {
        let (_, handle) = health_service();
        let map = handle.statuses.read().await;
        assert_eq!(map.get(""), Some(&ServingStatus::Serving));
    }

    #[tokio::test]
    async fn check_unknown_service() {
        let (server, _) = health_service();
        let mut svc = CheckSvc {
            statuses: server.statuses.clone(),
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
            statuses: server.statuses.clone(),
        };
        let req = Request::new(HealthCheckRequest {
            service: "my.Svc".into(),
        });
        let resp = svc.call(req).await.unwrap();
        assert_eq!(resp.get_ref().status, ServingStatus::Serving as i32);
    }
}
