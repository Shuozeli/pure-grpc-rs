//! Integration tests: server-side mTLS client-certificate verification.
#![cfg(feature = "tls")]

use grpc_core::body::Body;
use grpc_server::{ClientAuth, PeerCertificates, Server};
use http::{Request, Response};
use rcgen::{BasicConstraints, Certificate, CertificateParams, DnType, IsCa, KeyPair};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::task::{Context, Poll};
use tokio::net::TcpListener;
use tower_service::Service;

/// Echoes back, via response headers, whether the connection carried a
/// verified client certificate (and its leaf, hex-encoded, if so).
#[derive(Clone)]
struct PeerCertProbe;

impl Service<Request<Body>> for PeerCertProbe {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Response<Body>, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let mut resp = Response::new(Body::empty());
        match req.extensions().get::<PeerCertificates>() {
            Some(certs) => {
                resp.headers_mut()
                    .insert("x-peer-cert-present", "true".parse().unwrap());
                if let Some(leaf) = certs.leaf() {
                    resp.headers_mut()
                        .insert("x-peer-cert-leaf-hex", hex_encode(leaf).parse().unwrap());
                }
            }
            None => {
                resp.headers_mut()
                    .insert("x-peer-cert-present", "false".parse().unwrap());
            }
        }
        std::future::ready(Ok(resp))
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn generate_ca(cn: &str) -> (Certificate, KeyPair, Vec<u8>) {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, cn);
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let pem = cert.pem().into_bytes();
    (cert, key, pem)
}

/// Returns (cert_pem, key_pem, cert_der).
fn generate_leaf(
    ca_cert: &Certificate,
    ca_key: &KeyPair,
    cn: &str,
    sans: Vec<String>,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut params = CertificateParams::new(sans).unwrap();
    params.distinguished_name.push(DnType::CommonName, cn);
    let key = KeyPair::generate().unwrap();
    let cert = params.signed_by(&key, ca_cert, ca_key).unwrap();
    (
        cert.pem().into_bytes(),
        key.serialize_pem().into_bytes(),
        cert.der().as_ref().to_vec(),
    )
}

/// Starts a PeerCertProbe server with the given mTLS config, returns its address.
async fn start_server(
    server_cert_pem: &[u8],
    server_key_pem: &[u8],
    client_ca_pem: &[u8],
    mode: ClientAuth,
) -> SocketAddr {
    let server = Server::builder()
        .mtls(server_cert_pem, server_key_pem, client_ca_pem, mode)
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        server
            .serve_with_listener(listener, PeerCertProbe)
            .await
            .unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

fn client_tls_config(
    server_ca_pem: &[u8],
    client_cert_pem: Option<(&[u8], &[u8])>,
) -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut std::io::BufReader::new(server_ca_pem)) {
        roots.add(cert.unwrap()).unwrap();
    }
    let builder = rustls::ClientConfig::builder().with_root_certificates(roots);

    match client_cert_pem {
        Some((cert_pem, key_pem)) => {
            let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(cert_pem))
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(key_pem))
                .unwrap()
                .unwrap();
            builder.with_client_auth_cert(certs, key).unwrap()
        }
        None => builder.with_no_client_auth(),
    }
}

async fn send_probe_request(
    addr: SocketAddr,
    tls_config: rustls::ClientConfig,
) -> Result<Response<Body>, grpc_core::BoxError> {
    let uri: http::Uri = format!("https://127.0.0.1:{}/probe", addr.port())
        .parse()
        .unwrap();
    let mut channel = grpc_client::Channel::connect_with_tls_config(uri.clone(), tls_config)
        .await
        .unwrap();
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    channel.call(req).await
}

#[tokio::test]
async fn required_mode_with_valid_client_cert_succeeds_and_carries_identity() {
    let (server_ca_cert, server_ca_key, server_ca_pem) = generate_ca("Server CA");
    let (server_cert, server_key, _) = generate_leaf(
        &server_ca_cert,
        &server_ca_key,
        "localhost",
        vec!["localhost".to_string(), "127.0.0.1".to_string()],
    );

    let (client_ca_cert, client_ca_key, client_ca_pem) = generate_ca("Client CA");
    let (client_cert, client_key, client_cert_der) =
        generate_leaf(&client_ca_cert, &client_ca_key, "device-1", vec![]);

    let addr = start_server(
        &server_cert,
        &server_key,
        &client_ca_pem,
        ClientAuth::Required,
    )
    .await;

    let tls_config = client_tls_config(&server_ca_pem, Some((&client_cert, &client_key)));
    let resp = send_probe_request(addr, tls_config).await.unwrap();

    assert_eq!(resp.headers().get("x-peer-cert-present").unwrap(), "true");
    assert_eq!(
        resp.headers().get("x-peer-cert-leaf-hex").unwrap(),
        &hex_encode(&client_cert_der)
    );
}

#[tokio::test]
async fn required_mode_without_client_cert_fails_handshake() {
    let (server_ca_cert, server_ca_key, server_ca_pem) = generate_ca("Server CA");
    let (server_cert, server_key, _) = generate_leaf(
        &server_ca_cert,
        &server_ca_key,
        "localhost",
        vec!["localhost".to_string(), "127.0.0.1".to_string()],
    );
    let (_client_ca_cert, _client_ca_key, client_ca_pem) = generate_ca("Client CA");

    let addr = start_server(
        &server_cert,
        &server_key,
        &client_ca_pem,
        ClientAuth::Required,
    )
    .await;

    let tls_config = client_tls_config(&server_ca_pem, None);
    let result = send_probe_request(addr, tls_config).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn required_mode_with_untrusted_client_cert_fails_handshake() {
    let (server_ca_cert, server_ca_key, server_ca_pem) = generate_ca("Server CA");
    let (server_cert, server_key, _) = generate_leaf(
        &server_ca_cert,
        &server_ca_key,
        "localhost",
        vec!["localhost".to_string(), "127.0.0.1".to_string()],
    );
    let (_trusted_ca_cert, _trusted_ca_key, trusted_client_ca_pem) =
        generate_ca("Trusted Client CA");

    // Client presents a cert signed by a DIFFERENT CA than the one the
    // server was configured to trust.
    let (untrusted_ca_cert, untrusted_ca_key, _untrusted_ca_pem) = generate_ca("Untrusted CA");
    let (client_cert, client_key, _) =
        generate_leaf(&untrusted_ca_cert, &untrusted_ca_key, "intruder", vec![]);

    let addr = start_server(
        &server_cert,
        &server_key,
        &trusted_client_ca_pem,
        ClientAuth::Required,
    )
    .await;

    let tls_config = client_tls_config(&server_ca_pem, Some((&client_cert, &client_key)));
    let result = send_probe_request(addr, tls_config).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn optional_mode_without_client_cert_succeeds_with_no_identity() {
    let (server_ca_cert, server_ca_key, server_ca_pem) = generate_ca("Server CA");
    let (server_cert, server_key, _) = generate_leaf(
        &server_ca_cert,
        &server_ca_key,
        "localhost",
        vec!["localhost".to_string(), "127.0.0.1".to_string()],
    );
    let (_client_ca_cert, _client_ca_key, client_ca_pem) = generate_ca("Client CA");

    let addr = start_server(
        &server_cert,
        &server_key,
        &client_ca_pem,
        ClientAuth::Optional,
    )
    .await;

    let tls_config = client_tls_config(&server_ca_pem, None);
    let resp = send_probe_request(addr, tls_config).await.unwrap();

    assert_eq!(resp.headers().get("x-peer-cert-present").unwrap(), "false");
}

#[tokio::test]
async fn optional_mode_with_valid_client_cert_carries_identity() {
    let (server_ca_cert, server_ca_key, server_ca_pem) = generate_ca("Server CA");
    let (server_cert, server_key, _) = generate_leaf(
        &server_ca_cert,
        &server_ca_key,
        "localhost",
        vec!["localhost".to_string(), "127.0.0.1".to_string()],
    );
    let (client_ca_cert, client_ca_key, client_ca_pem) = generate_ca("Client CA");
    let (client_cert, client_key, client_cert_der) =
        generate_leaf(&client_ca_cert, &client_ca_key, "device-1", vec![]);

    let addr = start_server(
        &server_cert,
        &server_key,
        &client_ca_pem,
        ClientAuth::Optional,
    )
    .await;

    let tls_config = client_tls_config(&server_ca_pem, Some((&client_cert, &client_key)));
    let resp = send_probe_request(addr, tls_config).await.unwrap();

    assert_eq!(resp.headers().get("x-peer-cert-present").unwrap(), "true");
    assert_eq!(
        resp.headers().get("x-peer-cert-leaf-hex").unwrap(),
        &hex_encode(&client_cert_der)
    );
}
