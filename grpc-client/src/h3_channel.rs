use bytes::Buf;
use grpc_core::body::Body;
use grpc_core::BoxError;
use grpc_core::BoxFuture;
use http::{Request, Response};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower_service::Service;

/// A gRPC channel over HTTP/3 (QUIC).
///
/// Wraps a quinn QUIC connection and sends gRPC requests via h3.
/// Cloning is cheap — all clones share the same underlying QUIC connection.
#[derive(Clone)]
pub struct H3Channel {
    conn: quinn::Connection,
    timeout: Option<std::time::Duration>,
}

impl H3Channel {
    /// Connect to a gRPC server over HTTP/3.
    ///
    /// `ca_pem` is the PEM-encoded CA certificate for verifying the server.
    /// Use `None` for system-native CA certificates.
    pub async fn connect(
        uri: http::Uri,
        ca_pem: Option<&[u8]>,
    ) -> Result<Self, BoxError> {
        let host = uri.host().ok_or("URI must have a host")?;
        let port = uri.port_u16().unwrap_or(443);
        let addr = tokio::net::lookup_host(format!("{}:{}", host, port))
            .await?
            .next()
            .ok_or("failed to resolve host")?;

        let tls_config = build_client_tls_config(ca_pem)?;
        // Bind to same address family as the target.
        let bind_addr: std::net::SocketAddr = if addr.is_ipv6() {
            "[::]:0".parse()?
        } else {
            "0.0.0.0:0".parse()?
        };
        let mut endpoint = quinn::Endpoint::client(bind_addr)?;
        endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
        )));

        tracing::debug!("H3Channel: connecting to {} (resolved: {})", uri, addr);
        let conn = endpoint.connect(addr, host)?.await?;
        tracing::debug!("H3Channel: QUIC connection established");

        Ok(Self {
            conn,
            timeout: None,
        })
    }

    /// Set a per-request timeout.
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Returns the remote address of the QUIC connection.
    pub fn remote_address(&self) -> std::net::SocketAddr {
        self.conn.remote_address()
    }
}

fn build_client_tls_config(
    ca_pem: Option<&[u8]>,
) -> Result<rustls::ClientConfig, BoxError> {
    let mut root_store = rustls::RootCertStore::empty();

    if let Some(ca_pem) = ca_pem {
        let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(ca_pem))
            .collect::<Result<Vec<_>, _>>()?;
        for cert in certs {
            root_store.add(cert)?;
        }
    } else {
        let native_certs = rustls_native_certs::load_native_certs();
        for cert in native_certs.certs {
            root_store.add(cert)?;
        }
    }

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h3".to_vec()];

    Ok(config)
}

impl Service<Request<Body>> for H3Channel {
    type Response = Response<Body>;
    type Error = BoxError;
    type Future = BoxFuture<Result<Response<Body>, BoxError>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let conn = self.conn.clone();
        let timeout = self.timeout;

        let fut = async move {
            send_h3_request(conn, req).await
        };

        match timeout {
            Some(duration) => Box::pin(async move {
                match tokio::time::timeout(duration, fut).await {
                    Ok(result) => result,
                    Err(_elapsed) => Err(Box::new(grpc_core::Status::deadline_exceeded(
                        "h3 channel request timed out",
                    )) as BoxError),
                }
            }),
            None => Box::pin(fut),
        }
    }
}

/// Send an HTTP request over h3 and return the response with a Body.
///
/// For bidi streaming: the request body is sent in a background task
/// so the caller can read the response stream concurrently.
async fn send_h3_request(
    conn: quinn::Connection,
    req: Request<Body>,
) -> Result<Response<Body>, BoxError> {
    use http_body::Body as HttpBody;

    let remote_addr = conn.remote_address();
    let (mut driver, mut send_request) =
        h3::client::new(h3_quinn::Connection::new(conn)).await?;

    // The driver must be polled to keep the connection alive.
    tokio::spawn(async move {
        let err = driver.wait_idle().await;
        tracing::debug!("h3 client driver closed: {}", err);
    });

    let (mut parts, body) = req.into_parts();
    // h3 requires HTTP/3 version and full URI (with scheme + authority).
    parts.version = http::Version::HTTP_3;
    parts.headers.remove("te"); // HTTP/2-specific header, not valid in HTTP/3.

    // The gRPC client's prepare_request strips the URI to path-only form
    // (e.g., "/ServiceName/Method"). HTTP/3 requires :scheme and :authority
    // pseudo-headers, which h3 derives from the full URI. Reconstruct it.
    if parts.uri.scheme().is_none() {
        let authority = remote_addr;
        let path = parts.uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
        parts.uri = http::Uri::builder()
            .scheme("https")
            .authority(authority.to_string())
            .path_and_query(path)
            .build()
            .map_err(|e| -> BoxError { Box::new(e) })?;
    }

    let h3_req = http::Request::from_parts(parts, ());

    let stream = send_request.send_request(h3_req).await?;

    // Split the bidi stream so we can send the request body and read
    // the response concurrently (required for bidi streaming RPCs).
    let (send_stream, recv_stream) = stream.split();

    // Spawn a background task to stream the request body.
    tokio::spawn(async move {
        let mut send = send_stream;
        let mut body = std::pin::pin!(body);
        loop {
            match std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
                Some(Ok(frame)) => {
                    if frame.is_data() {
                        let data = frame.into_data().unwrap();
                        if send.send_data(data).await.is_err() {
                            break;
                        }
                    } else if frame.is_trailers() {
                        let trailers = frame.into_trailers().unwrap();
                        let _ = send.send_trailers(trailers).await;
                        break;
                    }
                }
                Some(Err(_)) => break,
                None => break,
            }
        }
        let _ = send.finish().await;
    });

    // Read the response headers from the recv side.
    let mut recv = recv_stream;
    let resp = recv.recv_response().await?;
    let (resp_parts, ()) = resp.into_parts();

    // Wrap the recv stream as a Body.
    let recv_body = H3ClientRecvBody {
        recv,
        _send_request: send_request,
        data_done: false,
        trailers_done: false,
    };
    let body = Body::new(recv_body);

    Ok(http::Response::from_parts(resp_parts, body))
}

/// An HTTP Body that reads response data from an h3 client recv stream.
///
/// Holds `_send_request` to keep the h3 connection alive until this body is dropped.
struct H3ClientRecvBody {
    recv: h3::client::RequestStream<h3_quinn::RecvStream, bytes::Bytes>,
    _send_request: h3::client::SendRequest<h3_quinn::OpenStreams, bytes::Bytes>,
    data_done: bool,
    trailers_done: bool,
}

// h3::client::RequestStream<h3_quinn::RecvStream, Bytes> is Send.
unsafe impl Send for H3ClientRecvBody {}

impl http_body::Body for H3ClientRecvBody {
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
                    let data = buf.copy_to_bytes(buf.remaining());
                    return Poll::Ready(Some(Ok(http_body::Frame::data(data))));
                }
                Poll::Ready(Ok(None)) => {
                    this.data_done = true;
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Poll::Ready(Err(err)) => {
                    return Poll::Ready(Some(Err(grpc_core::Status::internal(format!(
                        "h3 client recv error: {}",
                        err
                    )))));
                }
                Poll::Pending => return Poll::Pending,
            }
        }

        // Data done — poll for trailers.
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
                    "h3 client trailers error: {}",
                    err
                )))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl std::fmt::Debug for H3Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("H3Channel")
            .field("remote", &self.conn.remote_address())
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h3_channel_is_clone() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<H3Channel>();
    }

    #[test]
    fn h3_channel_is_debug() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<H3Channel>();
    }
}
