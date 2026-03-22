use crate::Channel;
use http::Uri;
use std::time::Duration;

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// A builder for configuring and connecting a gRPC [`Channel`].
#[derive(Debug, Clone)]
pub struct Endpoint {
    uri: Uri,
    timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
    http2: Http2Config,
    #[cfg(feature = "tls")]
    tls: TlsMode,
}

/// HTTP/2 connection-level settings for the client.
#[derive(Debug, Clone, Default)]
pub(crate) struct Http2Config {
    pub initial_stream_window_size: Option<u32>,
    pub initial_connection_window_size: Option<u32>,
    pub adaptive_window: Option<bool>,
    pub max_frame_size: Option<u32>,
    pub keep_alive_interval: Option<Duration>,
    pub keep_alive_timeout: Option<Duration>,
}

#[cfg(feature = "tls")]
#[derive(Debug, Clone)]
enum TlsMode {
    None,
    Native,
    CustomCa(Vec<u8>),
}

impl Endpoint {
    pub fn new(uri: Uri) -> Self {
        #[cfg(not(feature = "tls"))]
        if uri.scheme_str() == Some("https") {
            panic!(
                "https:// URI requires the `tls` feature. \
                 Enable it with: grpc-client = {{ features = [\"tls\"] }}"
            );
        }

        #[cfg(feature = "tls")]
        let tls = if uri_is_https(&uri) {
            TlsMode::Native
        } else {
            TlsMode::None
        };

        Self {
            uri,
            timeout: None,
            connect_timeout: None,
            http2: Http2Config::default(),
            #[cfg(feature = "tls")]
            tls,
        }
    }

    pub fn from_static(s: &'static str) -> Self {
        Self::new(s.parse().expect("valid URI"))
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        assert!(!timeout.is_zero(), "timeout must be non-zero");
        self.timeout = Some(timeout);
        self
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        assert!(!timeout.is_zero(), "connect_timeout must be non-zero");
        self.connect_timeout = Some(timeout);
        self
    }

    /// Set the initial HTTP/2 stream-level flow control window size (bytes).
    pub fn initial_stream_window_size(mut self, size: u32) -> Self {
        self.http2.initial_stream_window_size = Some(size);
        self
    }

    /// Set the initial HTTP/2 connection-level flow control window size (bytes).
    pub fn initial_connection_window_size(mut self, size: u32) -> Self {
        self.http2.initial_connection_window_size = Some(size);
        self
    }

    /// Enable HTTP/2 adaptive flow control.
    pub fn http2_adaptive_window(mut self, enabled: bool) -> Self {
        self.http2.adaptive_window = Some(enabled);
        self
    }

    /// Set the maximum HTTP/2 frame size (bytes).
    pub fn max_frame_size(mut self, size: u32) -> Self {
        self.http2.max_frame_size = Some(size);
        self
    }

    /// Set the HTTP/2 keep-alive interval.
    pub fn http2_keep_alive_interval(mut self, interval: Duration) -> Self {
        self.http2.keep_alive_interval = Some(interval);
        self
    }

    /// Set the HTTP/2 keep-alive timeout.
    pub fn http2_keep_alive_timeout(mut self, timeout: Duration) -> Self {
        self.http2.keep_alive_timeout = Some(timeout);
        self
    }

    /// Enable TLS with system native CA certificates.
    #[cfg(feature = "tls")]
    pub fn tls(mut self) -> Self {
        self.tls = TlsMode::Native;
        self
    }

    /// Enable TLS with a custom CA certificate (PEM-encoded).
    #[cfg(feature = "tls")]
    pub fn tls_with_ca(mut self, ca_pem: Vec<u8>) -> Self {
        self.tls = TlsMode::CustomCa(ca_pem);
        self
    }

    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    /// Connect to the endpoint and return a [`Channel`].
    ///
    /// Applies `connect_timeout` (if set) to the connection establishment,
    /// and `timeout` (if set) as a per-request deadline on the returned channel.
    pub async fn connect(&self) -> Result<Channel, BoxError> {
        let uri = self.uri.clone();
        let http2 = self.http2.clone();
        let connect_fut = async {
            #[cfg(feature = "tls")]
            {
                match &self.tls {
                    TlsMode::None => Channel::connect_with_h2_config(uri, http2).await,
                    TlsMode::Native => Channel::connect_tls_with_h2_config(uri, http2).await,
                    TlsMode::CustomCa(ca) => {
                        Channel::connect_with_ca_and_h2_config(uri, ca, http2).await
                    }
                }
            }
            #[cfg(not(feature = "tls"))]
            {
                Channel::connect_with_h2_config(uri, http2).await
            }
        };

        let mut channel = match self.connect_timeout {
            Some(duration) => tokio::time::timeout(duration, connect_fut)
                .await
                .map_err(|_| -> BoxError {
                    Box::new(grpc_core::Status::deadline_exceeded("connect timed out"))
                })
                .and_then(|r| r)?,
            None => connect_fut.await?,
        };

        if let Some(timeout) = self.timeout {
            channel = channel.with_timeout(timeout);
        }

        Ok(channel)
    }
}

#[cfg(feature = "tls")]
fn uri_is_https(uri: &Uri) -> bool {
    uri.scheme_str() == Some("https")
}

impl TryFrom<&str> for Endpoint {
    type Error = http::uri::InvalidUri;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Ok(Self::new(s.parse()?))
    }
}

impl TryFrom<String> for Endpoint {
    type Error = http::uri::InvalidUri;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Ok(Self::new(s.parse()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_from_static() {
        let ep = Endpoint::from_static("http://localhost:50051");
        assert_eq!(ep.uri().host(), Some("localhost"));
        assert_eq!(ep.uri().port_u16(), Some(50051));
    }

    #[test]
    fn endpoint_try_from_str() {
        let ep: Endpoint = "http://127.0.0.1:8080".try_into().unwrap();
        assert_eq!(ep.uri().host(), Some("127.0.0.1"));
    }

    #[test]
    fn endpoint_builder_methods() {
        let ep = Endpoint::from_static("http://localhost:50051")
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(1));
        assert_eq!(ep.timeout, Some(Duration::from_secs(5)));
        assert_eq!(ep.connect_timeout, Some(Duration::from_secs(1)));
    }

    #[test]
    fn endpoint_http2_options() {
        let ep = Endpoint::from_static("http://localhost:50051")
            .initial_stream_window_size(1024 * 1024)
            .initial_connection_window_size(2 * 1024 * 1024)
            .http2_adaptive_window(true)
            .max_frame_size(32768);
        assert_eq!(ep.http2.initial_stream_window_size, Some(1024 * 1024));
        assert_eq!(
            ep.http2.initial_connection_window_size,
            Some(2 * 1024 * 1024)
        );
        assert_eq!(ep.http2.adaptive_window, Some(true));
        assert_eq!(ep.http2.max_frame_size, Some(32768));
    }

    #[tokio::test]
    async fn endpoint_connect_returns_channel() {
        let ep = Endpoint::from_static("http://127.0.0.1:50051");
        let channel = ep.connect().await;
        assert!(channel.is_ok());
        assert_eq!(channel.unwrap().uri().host(), Some("127.0.0.1"));
    }

    #[tokio::test]
    async fn endpoint_connect_propagates_timeout() {
        let ep = Endpoint::from_static("http://127.0.0.1:50051").timeout(Duration::from_secs(5));
        let channel = ep.connect().await.unwrap();
        assert_eq!(channel.uri().host(), Some("127.0.0.1"));
    }
}
