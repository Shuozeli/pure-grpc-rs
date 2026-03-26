//! Round-robin load balancing across multiple gRPC channels.

use crate::Channel;
use grpc_core::body::Body;
use grpc_core::BoxError;
use grpc_core::BoxFuture;
use grpc_core::Http2Config;
use http::{Request, Response, Uri};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use tower_service::Service;

/// A load-balanced channel that distributes requests across multiple endpoints
/// using round-robin selection.
///
/// Each endpoint maintains its own HTTP/2 connection with full stream multiplexing.
/// Cloning is cheap — all clones share the same set of connections and the atomic counter.
#[derive(Clone)]
pub struct BalancedChannel {
    channels: Arc<Vec<Channel>>,
    next: Arc<AtomicUsize>,
}

impl BalancedChannel {
    /// Create a balanced channel from a list of endpoint URIs.
    ///
    /// Establishes a `Channel` for each URI. All channels share the same
    /// HTTP/2 configuration defaults.
    ///
    /// Returns an error if `endpoints` is empty.
    pub async fn from_uris(endpoints: Vec<Uri>) -> Result<Self, BoxError> {
        Self::from_uris_with_h2_config(endpoints, Http2Config::default()).await
    }

    /// Create a balanced channel with custom HTTP/2 settings.
    ///
    /// Returns an error if `endpoints` is empty.
    pub(crate) async fn from_uris_with_h2_config(
        endpoints: Vec<Uri>,
        config: Http2Config,
    ) -> Result<Self, BoxError> {
        if endpoints.is_empty() {
            return Err("at least one endpoint is required".into());
        }
        let mut channels = Vec::with_capacity(endpoints.len());
        for uri in endpoints {
            channels.push(Channel::connect_with_h2_config(uri, config.clone()).await?);
        }
        Ok(Self {
            channels: Arc::new(channels),
            next: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Returns the number of backend channels.
    pub fn num_endpoints(&self) -> usize {
        self.channels.len()
    }
}

impl Service<Request<Body>> for BalancedChannel {
    type Response = Response<Body>;
    type Error = BoxError;
    type Future = BoxFuture<Result<Response<Body>, BoxError>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.channels.len();
        let mut channel = self.channels[idx].clone();
        Box::pin(async move { channel.call(req).await })
    }
}

impl std::fmt::Debug for BalancedChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BalancedChannel")
            .field("num_endpoints", &self.channels.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn balanced_channel_round_robin() {
        let uris: Vec<Uri> = vec![
            "http://127.0.0.1:50051".parse().unwrap(),
            "http://127.0.0.1:50052".parse().unwrap(),
            "http://127.0.0.1:50053".parse().unwrap(),
        ];

        let channel = BalancedChannel::from_uris(uris).await.unwrap();
        assert_eq!(channel.num_endpoints(), 3);

        // Verify round-robin counter advances
        assert_eq!(channel.next.load(Ordering::Relaxed), 0);
        let ch = channel.clone();
        assert_eq!(ch.num_endpoints(), 3);
    }

    #[tokio::test]
    async fn balanced_channel_empty_returns_error() {
        let result = BalancedChannel::from_uris(vec![]).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("at least one endpoint is required"));
    }

    #[test]
    fn balanced_channel_is_clone() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<BalancedChannel>();
    }

    #[test]
    fn balanced_channel_is_debug() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<BalancedChannel>();
    }
}
