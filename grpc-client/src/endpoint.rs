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
    #[cfg(feature = "tls")]
    tls: TlsMode,
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
            #[cfg(feature = "tls")]
            tls,
        }
    }

    pub fn from_static(s: &'static str) -> Self {
        Self::new(s.parse().expect("valid URI"))
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
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
    pub async fn connect(&self) -> Result<Channel, BoxError> {
        // TODO(refactor): apply timeout and connect_timeout settings
        #[cfg(feature = "tls")]
        {
            match &self.tls {
                TlsMode::None => Channel::connect(self.uri.clone()).await,
                TlsMode::Native => Channel::connect_tls(self.uri.clone()).await,
                TlsMode::CustomCa(ca) => Channel::connect_with_ca(self.uri.clone(), ca).await,
            }
        }
        #[cfg(not(feature = "tls"))]
        {
            Channel::connect(self.uri.clone()).await
        }
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
}
