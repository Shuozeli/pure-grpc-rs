use grpc_core::body::Body;
use http::{Request, Response};
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::Semaphore;
use tower_service::Service;
use tracing::{debug, info, trace, warn};

/// A gRPC server that listens on a UDP socket and serves HTTP/3 (QUIC) requests.
///
/// QUIC mandates TLS 1.3 — there is no plaintext mode. You must provide
/// PEM-encoded certificate and key for every call to `serve`.
const DEFAULT_CONCURRENCY_LIMIT: usize = 1024;

pub struct H3Server {
    timeout: Option<Duration>,
    concurrency_limit: usize,
}

impl H3Server {
    /// Create a new HTTP/3 server builder.
    pub fn builder() -> Self {
        Self {
            timeout: None,
            concurrency_limit: DEFAULT_CONCURRENCY_LIMIT,
        }
    }

    /// Set a per-request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the maximum number of concurrent QUIC connections.
    pub fn concurrency_limit(mut self, limit: usize) -> Self {
        assert!(limit > 0, "concurrency_limit must be > 0");
        self.concurrency_limit = limit;
        self
    }

    /// Serve the given service on the specified address.
    ///
    /// `cert_pem` and `key_pem` are PEM-encoded TLS certificate and private key.
    pub async fn serve<S>(
        self,
        addr: SocketAddr,
        svc: S,
        cert_pem: &[u8],
        key_pem: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
            + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
    {
        self.serve_with_shutdown(addr, svc, cert_pem, key_pem, std::future::pending())
            .await
    }

    /// Create a QUIC endpoint without starting the accept loop.
    ///
    /// Returns the endpoint so the caller can discover the bound address via
    /// `endpoint.local_addr()`. Pass the endpoint to [`serve_endpoint`] to start.
    pub fn bind(
        addr: SocketAddr,
        cert_pem: &[u8],
        key_pem: &[u8],
    ) -> Result<quinn::Endpoint, Box<dyn std::error::Error + Send + Sync>> {
        create_quic_endpoint(addr, cert_pem, key_pem)
    }

    /// Serve using a pre-created QUIC endpoint.
    pub async fn serve_endpoint<S>(
        self,
        endpoint: quinn::Endpoint,
        svc: S,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
            + Clone
            + Send
            + 'static,
        S::Future: Send + 'static,
    {
        self.serve_endpoint_with_shutdown(endpoint, svc, std::future::pending())
            .await
    }

    /// Serve using a pre-created QUIC endpoint with graceful shutdown.
    pub async fn serve_endpoint_with_shutdown<S, F>(
        self,
        endpoint: quinn::Endpoint,
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
        info!("gRPC/HTTP3 server listening on {}", endpoint.local_addr()?);
        self.run_accept_loop(endpoint, svc, signal).await
    }

    /// Serve with a graceful shutdown signal.
    pub async fn serve_with_shutdown<S, F>(
        self,
        addr: SocketAddr,
        svc: S,
        cert_pem: &[u8],
        key_pem: &[u8],
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
        let endpoint = create_quic_endpoint(addr, cert_pem, key_pem)?;
        info!("gRPC/HTTP3 server listening on {}", addr);
        self.run_accept_loop(endpoint, svc, signal).await
    }

    async fn run_accept_loop<S, F>(
        self,
        endpoint: quinn::Endpoint,
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
        let sem = Arc::new(Semaphore::new(self.concurrency_limit));
        let timeout = self.timeout;
        tokio::pin!(signal);

        loop {
            tokio::select! {
                incoming = endpoint.accept() => {
                    let Some(incoming) = incoming else {
                        debug!("QUIC endpoint closed");
                        break;
                    };

                    let sem_clone = sem.clone();
                    let permit = match sem_clone.try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            debug!("concurrency limit reached, waiting for a slot");
                            match sem.clone().acquire_owned().await {
                                Ok(permit) => permit,
                                Err(_) => {
                                    warn!("semaphore closed, stopping accept loop");
                                    break;
                                }
                            }
                        }
                    };

                    let svc = svc.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        match incoming.await {
                            Ok(conn) => {
                                let remote_addr = conn.remote_address();
                                debug!("accepted QUIC connection from {}", remote_addr);
                                if let Err(err) = serve_h3_connection(conn, svc, timeout).await {
                                    debug!("h3 connection error from {}: {}", remote_addr, err);
                                }
                            }
                            Err(err) => {
                                warn!("QUIC handshake error: {}", err);
                            }
                        }
                    });
                }
                _ = &mut signal => {
                    trace!("shutdown signal received, stopping accept loop");
                    break;
                }
            }
        }

        endpoint.close(0u32.into(), b"server shutting down");
        Ok(())
    }
}

/// Create a QUIC endpoint with TLS from PEM-encoded cert and key.
fn create_quic_endpoint(
    addr: SocketAddr,
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<quinn::Endpoint, Box<dyn std::error::Error + Send + Sync>> {
    let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(cert_pem))
        .collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(key_pem))?
        .ok_or("no private key found in PEM data")?;

    let mut tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    tls_config.alpn_protocols = vec![b"h3".to_vec()];

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)?,
    ));

    let endpoint = quinn::Endpoint::server(server_config, addr)?;
    Ok(endpoint)
}

/// Handle a single QUIC connection: accept HTTP/3 requests and dispatch to the service.
async fn serve_h3_connection<S>(
    conn: quinn::Connection,
    svc: S,
    timeout: Option<Duration>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    let mut h3_conn = h3::server::Connection::new(h3_quinn::Connection::new(conn)).await?;

    loop {
        match h3_conn.accept().await {
            Ok(Some(resolver)) => {
                let svc = svc.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_h3_request(resolver, svc, timeout).await {
                        debug!("h3 request error: {}", err);
                    }
                });
            }
            Ok(None) => break,
            Err(err) => {
                warn!("h3 accept error: {}", err);
                break;
            }
        }
    }

    Ok(())
}

/// Handle a single HTTP/3 request: bridge h3 types to Service<Request<Body>>.
async fn handle_h3_request<S>(
    resolver: h3::server::RequestResolver<h3_quinn::Connection, bytes::Bytes>,
    mut svc: S,
    timeout: Option<Duration>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible> + Send,
    S::Future: Send,
{
    let (req, stream) = resolver.resolve_request().await?;

    // Split the bidi stream into send and recv halves.
    // This lets us read the request body while retaining the send half for the response.
    let (send_stream, recv_stream) = stream.split();

    // Build Request<Body> by wrapping the recv half as an HTTP Body.
    let (parts, ()) = req.into_parts();
    let recv_body = H3RecvBody::new(recv_stream);
    let body = Body::new(recv_body);
    let request = http::Request::from_parts(parts, body);

    // Call the service (with optional timeout).
    let response = match timeout {
        Some(duration) => match tokio::time::timeout(duration, svc.call(request)).await {
            Ok(result) => result.unwrap(), // Infallible
            Err(_elapsed) => {
                let status = grpc_core::Status::deadline_exceeded("request timed out");
                status.into_http()
            }
        },
        None => svc.call(request).await.unwrap(), // Infallible
    };

    // Send the response back via the h3 send half.
    send_h3_response(send_stream, response).await?;

    Ok(())
}

/// Stream a Response<Body> through an h3 send stream.
async fn send_h3_response(
    mut send: h3::server::RequestStream<h3_quinn::SendStream<bytes::Bytes>, bytes::Bytes>,
    response: Response<Body>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use http_body::Body as HttpBody;

    let (parts, body) = response.into_parts();
    let resp = http::Response::from_parts(parts, ());
    send.send_response(resp).await?;

    // Stream body frames (data + trailers).
    let mut body = std::pin::pin!(body);
    let mut trailers: Option<http::HeaderMap> = None;

    loop {
        match std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
            Some(Ok(frame)) => {
                // Frame can be data or trailers.
                if frame.is_data() {
                    // SAFETY: we just checked is_data().
                    let data = frame.into_data().unwrap();
                    send.send_data(data).await?;
                } else if frame.is_trailers() {
                    // SAFETY: we just checked is_trailers().
                    trailers = Some(frame.into_trailers().unwrap());
                }
            }
            Some(Err(status)) => {
                // gRPC error during body streaming — encode as trailers.
                let mut t = http::HeaderMap::new();
                if let Err(encode_err) = status.add_header(&mut t) {
                    warn!("failed to encode gRPC status to trailers: {}", encode_err);
                }
                trailers = Some(t);
                break;
            }
            None => break,
        }
    }

    if let Some(t) = trailers {
        send.send_trailers(t).await?;
    }

    send.finish().await?;
    Ok(())
}

/// An HTTP Body that reads data from an h3 recv stream.
///
/// Bridges h3's recv_data/recv_trailers into the `http_body::Body` trait
/// that the existing gRPC codec layer expects.
struct H3RecvBody {
    recv: h3::server::RequestStream<h3_quinn::RecvStream, bytes::Bytes>,
    data_done: bool,
    trailers_done: bool,
}

impl H3RecvBody {
    fn new(recv: h3::server::RequestStream<h3_quinn::RecvStream, bytes::Bytes>) -> Self {
        Self {
            recv,
            data_done: false,
            trailers_done: false,
        }
    }
}


impl http_body::Body for H3RecvBody {
    type Data = bytes::Bytes;
    type Error = grpc_core::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();

        if this.trailers_done {
            return Poll::Ready(None);
        }

        if !this.data_done {
            match this.recv.poll_recv_data(cx) {
                Poll::Ready(Ok(Some(mut buf))) => {
                    // Convert the h3 buffer (impl Buf) to Bytes.
                    use bytes::Buf;
                    let data = buf.copy_to_bytes(buf.remaining());
                    return Poll::Ready(Some(Ok(http_body::Frame::data(data))));
                }
                Poll::Ready(Ok(None)) => {
                    this.data_done = true;
                    // Wake to poll trailers on next call.
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Poll::Ready(Err(err)) => {
                    return Poll::Ready(Some(Err(grpc_core::Status::internal(format!(
                        "h3 recv error: {}",
                        err
                    )))));
                }
                Poll::Pending => return Poll::Pending,
            }
        }

        // Data is done — poll for trailers.
        match this.recv.poll_recv_trailers(cx) {
            Poll::Ready(Ok(Some(trailers))) => {
                this.trailers_done = true;
                Poll::Ready(Some(Ok(http_body::Frame::trailers(trailers))))
            }
            Poll::Ready(Ok(None)) => {
                this.trailers_done = true;
                Poll::Ready(None)
            }
            Poll::Ready(Err(err)) => {
                this.trailers_done = true;
                Poll::Ready(Some(Err(grpc_core::Status::internal(format!(
                    "h3 trailers error: {}",
                    err
                )))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h3_server_builder_defaults() {
        let server = H3Server::builder();
        assert!(server.timeout.is_none());
        assert_eq!(server.concurrency_limit, DEFAULT_CONCURRENCY_LIMIT);
    }

    #[test]
    fn h3_server_builder_options() {
        let server = H3Server::builder()
            .timeout(Duration::from_secs(30))
            .concurrency_limit(100);
        assert_eq!(server.timeout, Some(Duration::from_secs(30)));
        assert_eq!(server.concurrency_limit, 100);
    }

    #[test]
    #[should_panic(expected = "concurrency_limit must be > 0")]
    fn h3_server_concurrency_limit_zero_panics() {
        H3Server::builder().concurrency_limit(0);
    }
}
