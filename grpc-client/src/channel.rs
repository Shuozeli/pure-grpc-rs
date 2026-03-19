use grpc_core::body::Body;
use grpc_core::BoxFuture;
use http::{Request, Response};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::task::{Context, Poll};
use std::time::Duration;
use tower_service::Service;
type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// A gRPC channel (HTTP/2 client connection).
///
/// Cloning is cheap — all clones share the same underlying HTTP/2 connection.
///
/// For TLS connections, use `Channel::connect_tls` (requires `tls` feature).
#[derive(Clone)]
pub struct Channel {
    inner: ChannelInner,
    uri: http::Uri,
    timeout: Option<Duration>,
}

#[derive(Clone)]
enum ChannelInner {
    Http(Client<hyper_util::client::legacy::connect::HttpConnector, Body>),
    #[cfg(feature = "tls")]
    Https(
        Client<
            hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
            Body,
        >,
    ),
}

impl Channel {
    /// Create a new channel for plaintext HTTP/2 connections.
    ///
    /// The actual TCP connection is established lazily on first request.
    pub async fn connect(uri: http::Uri) -> Result<Self, BoxError> {
        let client = Client::builder(TokioExecutor::new())
            .http2_only(true)
            .build_http();
        Ok(Channel {
            inner: ChannelInner::Http(client),
            uri,
            timeout: None,
        })
    }

    /// Set a per-request timeout on this channel.
    ///
    /// Requests that exceed this duration return a `DeadlineExceeded` error.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Create a new channel for TLS-encrypted HTTP/2 connections.
    ///
    /// Uses the system's native certificate store for CA verification.
    #[cfg(feature = "tls")]
    pub async fn connect_tls(uri: http::Uri) -> Result<Self, BoxError> {
        let tls_config = Self::default_tls_config()?;
        Self::connect_with_tls_config(uri, tls_config).await
    }

    /// Create a new channel with a custom TLS config.
    ///
    /// Use this for custom CA certificates or client certificates.
    #[cfg(feature = "tls")]
    pub async fn connect_with_tls_config(
        uri: http::Uri,
        tls_config: rustls::ClientConfig,
    ) -> Result<Self, BoxError> {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls_config)
            .https_or_http()
            .enable_http2()
            .build();

        let client = Client::builder(TokioExecutor::new())
            .http2_only(true)
            .build(https);

        Ok(Channel {
            inner: ChannelInner::Https(client),
            uri,
            timeout: None,
        })
    }

    /// Create a new channel with a custom CA certificate (PEM-encoded).
    ///
    /// Useful for self-signed certificates in development/testing.
    #[cfg(feature = "tls")]
    pub async fn connect_with_ca(uri: http::Uri, ca_pem: &[u8]) -> Result<Self, BoxError> {
        let mut root_store = rustls::RootCertStore::empty();
        let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(ca_pem))
            .collect::<Result<Vec<_>, _>>()?;
        for cert in certs {
            root_store.add(cert)?;
        }

        let tls_config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Self::connect_with_tls_config(uri, tls_config).await
    }

    #[cfg(feature = "tls")]
    fn default_tls_config() -> Result<rustls::ClientConfig, BoxError> {
        let native_certs = rustls_native_certs::load_native_certs();
        let mut root_store = rustls::RootCertStore::empty();
        for cert in native_certs.certs {
            root_store.add(cert)?;
        }

        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(config)
    }

    /// Returns the URI this channel was created with.
    pub fn uri(&self) -> &http::Uri {
        &self.uri
    }
}

impl Service<Request<Body>> for Channel {
    type Response = Response<Body>;
    type Error = BoxError;
    type Future = BoxFuture<Result<Response<Body>, BoxError>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let timeout = self.timeout;

        let fut = match &self.inner {
            ChannelInner::Http(client) => {
                let client = client.clone();
                Box::pin(async move {
                    let resp = client
                        .request(req)
                        .await
                        .map_err(|e| -> BoxError { Box::new(e) })?;
                    Ok(resp.map(Body::new))
                }) as BoxFuture<Result<Response<Body>, BoxError>>
            }
            #[cfg(feature = "tls")]
            ChannelInner::Https(client) => {
                let client = client.clone();
                Box::pin(async move {
                    let resp = client
                        .request(req)
                        .await
                        .map_err(|e| -> BoxError { Box::new(e) })?;
                    Ok(resp.map(Body::new))
                })
            }
        };

        match timeout {
            Some(duration) => Box::pin(async move {
                match tokio::time::timeout(duration, fut).await {
                    Ok(result) => result,
                    Err(_elapsed) => Err(Box::new(grpc_core::Status::deadline_exceeded(
                        "channel request timed out",
                    )) as BoxError),
                }
            }),
            None => fut,
        }
    }
}

impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("uri", &self.uri)
            .field(
                "tls",
                &matches!(self.inner, ChannelInner::Http(_))
                    .then_some("no")
                    .unwrap_or("yes"),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_is_clone() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<Channel>();
    }

    #[test]
    fn channel_is_debug() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<Channel>();
    }
}
