use grpc_core::body::Body;
use http::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnectionBuilder;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tower_service::Service;
use tracing::{debug, info, trace};

/// A gRPC server that listens on a TCP socket and serves HTTP/2 requests.
///
/// Supports optional TLS via rustls (feature = "tls").
/// Default max concurrent connections if not configured.
const DEFAULT_CONCURRENCY_LIMIT: usize = 1024;

pub struct Server {
    tcp_nodelay: bool,
    timeout: Option<Duration>,
    concurrency_limit: usize,
    http2: Http2Config,
    #[cfg(feature = "tls")]
    tls_config: Option<TlsConfig>,
}

/// HTTP/2 connection-level settings.
#[derive(Clone, Default)]
struct Http2Config {
    initial_stream_window_size: Option<u32>,
    initial_connection_window_size: Option<u32>,
    adaptive_window: Option<bool>,
    max_frame_size: Option<u32>,
    max_concurrent_streams: Option<u32>,
    keep_alive_interval: Option<Duration>,
    keep_alive_timeout: Option<Duration>,
}

#[cfg(feature = "tls")]
struct TlsConfig {
    acceptor: tokio_rustls::TlsAcceptor,
}

impl Server {
    /// Create a new server builder.
    pub fn builder() -> Self {
        Self {
            tcp_nodelay: true,
            timeout: None,
            concurrency_limit: DEFAULT_CONCURRENCY_LIMIT,
            http2: Http2Config::default(),
            #[cfg(feature = "tls")]
            tls_config: None,
        }
    }

    pub fn tcp_nodelay(mut self, enabled: bool) -> Self {
        self.tcp_nodelay = enabled;
        self
    }

    /// Set a per-request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the maximum number of concurrent connections.
    ///
    /// Defaults to 1024. Once the limit is reached, the server stops accepting
    /// new connections until an existing connection closes.
    pub fn concurrency_limit(mut self, limit: usize) -> Self {
        assert!(limit > 0, "concurrency_limit must be > 0");
        self.concurrency_limit = limit;
        self
    }

    /// Set the initial HTTP/2 stream-level flow control window size (bytes).
    ///
    /// Larger values reduce stalls for large messages but consume more memory per stream.
    /// Default: 65,535 bytes (h2 spec default).
    pub fn initial_stream_window_size(mut self, size: u32) -> Self {
        self.http2.initial_stream_window_size = Some(size);
        self
    }

    /// Set the initial HTTP/2 connection-level flow control window size (bytes).
    ///
    /// Controls the aggregate flow across all streams on a connection.
    /// Default: 65,535 bytes (h2 spec default).
    pub fn initial_connection_window_size(mut self, size: u32) -> Self {
        self.http2.initial_connection_window_size = Some(size);
        self
    }

    /// Enable HTTP/2 adaptive flow control.
    ///
    /// When enabled, the connection dynamically adjusts window sizes based on
    /// observed bandwidth-delay product. Recommended for high-throughput streaming.
    pub fn http2_adaptive_window(mut self, enabled: bool) -> Self {
        self.http2.adaptive_window = Some(enabled);
        self
    }

    /// Set the maximum HTTP/2 frame size (bytes).
    ///
    /// Must be between 16,384 and 16,777,215 (h2 spec bounds).
    /// Default: 16,384 bytes.
    pub fn max_frame_size(mut self, size: u32) -> Self {
        self.http2.max_frame_size = Some(size);
        self
    }

    /// Set the maximum number of concurrent HTTP/2 streams per connection.
    ///
    /// Limits how many RPCs can be in-flight on a single connection.
    pub fn max_concurrent_streams(mut self, max: u32) -> Self {
        self.http2.max_concurrent_streams = Some(max);
        self
    }

    /// Set the HTTP/2 keep-alive interval.
    ///
    /// Sends PING frames at this interval to detect dead connections.
    pub fn http2_keep_alive_interval(mut self, interval: Duration) -> Self {
        self.http2.keep_alive_interval = Some(interval);
        self
    }

    /// Set the HTTP/2 keep-alive timeout.
    ///
    /// If a PING response is not received within this duration, the connection is closed.
    pub fn http2_keep_alive_timeout(mut self, timeout: Duration) -> Self {
        self.http2.keep_alive_timeout = Some(timeout);
        self
    }

    /// Configure TLS using PEM-encoded certificate and private key.
    #[cfg(feature = "tls")]
    pub fn tls(
        mut self,
        cert_pem: &[u8],
        key_pem: &[u8],
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        use rustls::ServerConfig;
        use std::sync::Arc;

        let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(cert_pem))
            .collect::<Result<Vec<_>, _>>()?;
        let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(key_pem))?
            .ok_or("no private key found in PEM data")?;

        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;

        self.tls_config = Some(TlsConfig {
            acceptor: tokio_rustls::TlsAcceptor::from(Arc::new(config)),
        });

        Ok(self)
    }

    /// Serve the given service on the specified address.
    pub async fn serve<S>(
        self,
        addr: SocketAddr,
        svc: S,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
            + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
    {
        let listener = TcpListener::bind(addr).await?;
        info!("gRPC server listening on {}", addr);

        self.serve_with_listener(listener, svc).await
    }

    /// Serve with a graceful shutdown signal.
    pub async fn serve_with_shutdown<S, F>(
        self,
        addr: SocketAddr,
        svc: S,
        signal: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
            + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
        F: Future<Output = ()> + Send,
    {
        let listener = TcpListener::bind(addr).await?;
        info!("gRPC server listening on {}", addr);

        self.serve_listener_with_shutdown(listener, svc, signal)
            .await
    }

    /// Serve using an already-bound `TcpListener`.
    pub async fn serve_with_listener<S>(
        self,
        listener: TcpListener,
        svc: S,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
            + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
    {
        self.serve_listener_with_shutdown(listener, svc, std::future::pending())
            .await
    }

    async fn serve_listener_with_shutdown<S, F>(
        self,
        listener: TcpListener,
        svc: S,
        signal: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
            + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
        F: Future<Output = ()> + Send,
    {
        let timeout = self.timeout;
        let http2 = Arc::new(self.http2);
        let sem = Arc::new(Semaphore::new(self.concurrency_limit));
        tokio::pin!(signal);

        loop {
            tokio::select! {
                result = listener.accept() => {
                    let (stream, remote_addr) = result?;

                    if self.tcp_nodelay {
                        stream.set_nodelay(true)?;
                    }

                    let sem_clone = sem.clone();
                    let permit = match sem_clone.try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            debug!("concurrency limit reached, waiting for a slot");
                            sem.clone().acquire_owned().await
                                .expect("semaphore should not be closed")
                        }
                    };

                    debug!("accepted connection from {}", remote_addr);

                    let svc = svc.clone();
                    let http2 = http2.clone();

                    #[cfg(feature = "tls")]
                    if let Some(ref tls_config) = self.tls_config {
                        let acceptor = tls_config.acceptor.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            match acceptor.accept(stream).await {
                                Ok(tls_stream) => {
                                    serve_connection(TokioIo::new(tls_stream), svc, timeout, &http2, remote_addr).await;
                                }
                                Err(err) => {
                                    tracing::warn!("TLS handshake error from {}: {}", remote_addr, err);
                                }
                            }
                        });
                        continue;
                    }

                    tokio::spawn(async move {
                        let _permit = permit;
                        serve_connection(TokioIo::new(stream), svc, timeout, &http2, remote_addr).await;
                    });
                }
                _ = &mut signal => {
                    trace!("shutdown signal received, stopping accept loop");
                    break;
                }
            }
        }

        Ok(())
    }
}

async fn serve_connection<I, S>(
    io: I,
    svc: S,
    timeout: Option<Duration>,
    http2: &Http2Config,
    remote_addr: SocketAddr,
) where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    let hyper_svc = HyperServiceWrapper {
        inner: svc,
        timeout,
    };
    let mut builder = ConnectionBuilder::new(TokioExecutor::new());

    // Apply HTTP/2 settings
    {
        let mut h2 = builder.http2();
        if let Some(sz) = http2.initial_stream_window_size {
            h2.initial_stream_window_size(sz);
        }
        if let Some(sz) = http2.initial_connection_window_size {
            h2.initial_connection_window_size(sz);
        }
        if let Some(enabled) = http2.adaptive_window {
            h2.adaptive_window(enabled);
        }
        if let Some(sz) = http2.max_frame_size {
            h2.max_frame_size(sz);
        }
        if let Some(max) = http2.max_concurrent_streams {
            h2.max_concurrent_streams(max);
        }
        if let Some(interval) = http2.keep_alive_interval {
            h2.keep_alive_interval(interval);
        }
        if let Some(timeout) = http2.keep_alive_timeout {
            h2.keep_alive_timeout(timeout);
        }
    }

    if let Err(err) = builder.serve_connection(io, hyper_svc).await {
        debug!("connection error from {}: {}", remote_addr, err);
    }
}

/// Adapter: wraps a `tower::Service<Request<Body>>` into a hyper service,
/// with optional per-request timeout enforcement.
#[derive(Clone)]
struct HyperServiceWrapper<S> {
    inner: S,
    timeout: Option<Duration>,
}

impl<S> hyper::service::Service<Request<hyper::body::Incoming>> for HyperServiceWrapper<S>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, Infallible>> + Send>>;

    fn call(&self, req: Request<hyper::body::Incoming>) -> Self::Future {
        let req = req.map(Body::new);
        let mut svc = self.inner.clone();
        let timeout = self.timeout;

        Box::pin(async move {
            match timeout {
                Some(duration) => match tokio::time::timeout(duration, svc.call(req)).await {
                    Ok(result) => result,
                    Err(_elapsed) => {
                        let status = grpc_core::Status::deadline_exceeded("request timed out");
                        Ok(status.into_http())
                    }
                },
                None => svc.call(req).await,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_builder_defaults() {
        let server = Server::builder();
        assert!(server.tcp_nodelay);
        assert!(server.timeout.is_none());
        assert_eq!(server.concurrency_limit, DEFAULT_CONCURRENCY_LIMIT);
        assert!(server.http2.initial_stream_window_size.is_none());
        assert!(server.http2.max_concurrent_streams.is_none());
    }

    #[test]
    fn server_builder_options() {
        let server = Server::builder()
            .tcp_nodelay(false)
            .timeout(Duration::from_secs(30));
        assert!(!server.tcp_nodelay);
        assert_eq!(server.timeout, Some(Duration::from_secs(30)));
    }

    #[test]
    fn server_builder_http2_options() {
        let server = Server::builder()
            .initial_stream_window_size(1024 * 1024)
            .initial_connection_window_size(2 * 1024 * 1024)
            .http2_adaptive_window(true)
            .max_frame_size(32768)
            .max_concurrent_streams(100)
            .http2_keep_alive_interval(Duration::from_secs(10))
            .http2_keep_alive_timeout(Duration::from_secs(20));
        assert_eq!(server.http2.initial_stream_window_size, Some(1024 * 1024));
        assert_eq!(
            server.http2.initial_connection_window_size,
            Some(2 * 1024 * 1024)
        );
        assert_eq!(server.http2.adaptive_window, Some(true));
        assert_eq!(server.http2.max_frame_size, Some(32768));
        assert_eq!(server.http2.max_concurrent_streams, Some(100));
        assert_eq!(
            server.http2.keep_alive_interval,
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            server.http2.keep_alive_timeout,
            Some(Duration::from_secs(20))
        );
    }

    #[tokio::test]
    async fn graceful_shutdown() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        let server_handle = tokio::spawn(async {
            let svc = MockService;
            Server::builder()
                .serve_with_shutdown("127.0.0.1:0".parse().unwrap(), svc, async {
                    rx.await.ok();
                })
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send(()).unwrap();

        let result = server_handle.await.unwrap();
        assert!(result.is_ok());
    }

    #[derive(Clone)]
    struct MockService;

    impl Service<Request<Body>> for MockService {
        type Response = Response<Body>;
        type Error = Infallible;
        type Future = std::future::Ready<Result<Response<Body>, Infallible>>;

        fn poll_ready(
            &mut self,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: Request<Body>) -> Self::Future {
            std::future::ready(Ok(Response::new(Body::empty())))
        }
    }
}
