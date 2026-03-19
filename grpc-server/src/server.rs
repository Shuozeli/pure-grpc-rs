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
    #[cfg(feature = "tls")]
    tls_config: Option<TlsConfig>,
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

                    #[cfg(feature = "tls")]
                    if let Some(ref tls_config) = self.tls_config {
                        let acceptor = tls_config.acceptor.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            match acceptor.accept(stream).await {
                                Ok(tls_stream) => {
                                    serve_connection(TokioIo::new(tls_stream), svc, timeout, remote_addr).await;
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
                        serve_connection(TokioIo::new(stream), svc, timeout, remote_addr).await;
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

async fn serve_connection<I, S>(io: I, svc: S, timeout: Option<Duration>, remote_addr: SocketAddr)
where
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
    let builder = ConnectionBuilder::new(TokioExecutor::new());
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
    }

    #[test]
    fn server_builder_options() {
        let server = Server::builder()
            .tcp_nodelay(false)
            .timeout(Duration::from_secs(30));
        assert!(!server.tcp_nodelay);
        assert_eq!(server.timeout, Some(Duration::from_secs(30)));
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
