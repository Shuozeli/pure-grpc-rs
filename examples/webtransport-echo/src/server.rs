/// WebTransport echo server for E2E browser testing.
///
/// Accepts WebTransport sessions, opens/accepts bidi streams,
/// and echoes received data back with a "echo:" prefix.
///
/// Also serves a simple HTML test page over HTTPS (regular HTTP/3).
use bytes::Bytes;
#[allow(unused_imports)]
use h3::ext::Protocol;
use h3::quic::BidiStream as _;
use h3_webtransport::server::WebTransportSession;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let addr: SocketAddr = "[::]:4433".parse()?;
    let (cert_pem, key_pem, cert_der) = generate_cert();

    // Compute cert hash for Chrome's serverCertificateHashes option.
    use sha2::Digest;
    let hash = sha2::Sha256::digest(&cert_der);
    use base64::Engine;
    let cert_hash_b64 = base64::engine::general_purpose::STANDARD.encode(hash);
    info!("Certificate SHA-256 (hex): {}", hex_encode(&hash));
    info!("Certificate SHA-256 (base64): {}", cert_hash_b64);

    let endpoint = create_endpoint(addr, &cert_pem, &key_pem)?;
    info!("WebTransport echo server listening on udp://{}", addr);
    info!("Open https://localhost:4433/ in Chrome to test");

    loop {
        let Some(incoming) = endpoint.accept().await else {
            break;
        };

        let cert_hash_b64 = cert_hash_b64.clone();
        tokio::spawn(async move {
            match incoming.await {
                Ok(conn) => {
                    let remote = conn.remote_address();
                    info!("QUIC connection from {}", remote);
                    if let Err(err) = handle_connection(conn, &cert_hash_b64).await {
                        error!("connection error from {}: {}", remote, err);
                    }
                }
                Err(err) => {
                    error!("QUIC handshake error: {}", err);
                }
            }
        });
    }

    Ok(())
}

async fn handle_connection(
    conn: quinn::Connection,
    cert_hash_b64: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Build h3 connection with WebTransport enabled.
    let mut h3_conn = h3::server::builder()
        .enable_webtransport(true)
        .enable_extended_connect(true)
        .enable_datagram(true)
        .max_webtransport_sessions(1)
        .send_grease(true)
        .build(h3_quinn::Connection::new(conn))
        .await?;

    loop {
        match h3_conn.accept().await {
            Ok(Some(resolver)) => {
                let (req, stream) = resolver.resolve_request().await?;
                debug!(
                    "request: {} {} {:?}",
                    req.method(),
                    req.uri(),
                    req.extensions().get::<Protocol>()
                );

                // Check if this is a WebTransport CONNECT request.
                let is_wt = req.method() == http::Method::CONNECT
                    && req
                        .extensions()
                        .get::<Protocol>()
                        .map(|p| p == &Protocol::WEB_TRANSPORT)
                        .unwrap_or(false);

                if is_wt {
                    info!("WebTransport session request: {}", req.uri());
                    let session = WebTransportSession::accept(req, stream, h3_conn).await?;
                    info!("WebTransport session established");
                    handle_wt_session(session).await?;
                    // After session ends, we can't accept more on this h3_conn
                    // (it was moved into the session). Break.
                    return Ok(());
                } else if req.method() == http::Method::GET {
                    // Serve the HTML test page.
                    serve_html(stream, cert_hash_b64).await?;
                } else {
                    // Unknown request.
                    let mut stream = stream;
                    let resp = http::Response::builder().status(404).body(()).unwrap();
                    stream.send_response(resp).await?;
                    stream.finish().await?;
                }
            }
            Ok(None) => break,
            Err(err) => {
                error!("h3 accept error: {}", err);
                break;
            }
        }
    }

    Ok(())
}

async fn handle_wt_session(
    session: WebTransportSession<h3_quinn::Connection, bytes::Bytes>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Accept bidi streams from the client and echo data back.
    loop {
        match session.accept_bi().await {
            Ok(Some(accepted)) => {
                let stream = match accepted {
                    h3_webtransport::server::AcceptedBi::BidiStream(_session_id, stream) => stream,
                    h3_webtransport::server::AcceptedBi::Request(_, _) => {
                        debug!("ignoring HTTP request on WebTransport session");
                        continue;
                    }
                };
                let (mut send, mut recv) = stream.split();
                info!("WebTransport bidi stream accepted");

                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 4096];
                    // Read data from client, echo with prefix.
                    loop {
                        match recv.read(&mut buf).await {
                            Ok(0) => {
                                debug!("bidi stream recv done");
                                break;
                            }
                            Ok(n) => {
                                let mut out = Vec::with_capacity(5 + n);
                                out.extend_from_slice(b"echo:");
                                out.extend_from_slice(&buf[..n]);
                                if let Err(err) = send.write_all(&out).await {
                                    debug!("send error: {}", err);
                                    break;
                                }
                            }
                            Err(err) => {
                                debug!("recv error: {}", err);
                                break;
                            }
                        }
                    }
                });
            }
            Ok(None) => {
                info!("WebTransport session closed");
                break;
            }
            Err(err) => {
                error!("WebTransport accept_bi error: {}", err);
                break;
            }
        }
    }

    Ok(())
}

async fn serve_html(
    mut stream: h3::server::RequestStream<h3_quinn::BidiStream<bytes::Bytes>, bytes::Bytes>,
    cert_hash_b64: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Inject the certificate hash into the HTML so the WebTransport constructor
    // can use serverCertificateHashes for self-signed cert validation.
    let html = include_str!("../static/index.html").replace("{{CERT_HASH_B64}}", cert_hash_b64);
    let resp = http::Response::builder()
        .status(200)
        .header("content-type", "text/html")
        .body(())
        .unwrap();
    stream.send_response(resp).await?;
    stream.send_data(Bytes::from(html)).await?;
    stream.finish().await?;
    Ok(())
}

fn generate_cert() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    // WebTransport with serverCertificateHashes requires the certificate to:
    // 1. Have a validity period of at most 14 days.
    // 2. Use an ECDSA key (P-256).
    use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};

    let mut params =
        CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
    // Set validity to 14 days (maximum for serverCertificateHashes).
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now + time::Duration::days(14);

    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let cert = params.self_signed(&key_pair).unwrap();

    let cert_der = cert.der().to_vec();
    let cert_pem = cert.pem().into_bytes();
    let key_pem = key_pair.serialize_pem().into_bytes();
    (cert_pem, key_pem, cert_der)
}

fn create_endpoint(
    addr: SocketAddr,
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<quinn::Endpoint, Box<dyn std::error::Error>> {
    let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(cert_pem))
        .collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(key_pem))?
        .ok_or("no private key found")?;

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

fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(":")
}
