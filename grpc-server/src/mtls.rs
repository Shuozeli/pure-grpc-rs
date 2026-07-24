//! Types for mutual TLS (mTLS) client-certificate authentication.
//!
//! See [`crate::Server::mtls`] for the server-side builder method that
//! populates [`PeerCertificates`].

use std::sync::Arc;

/// Client certificate verification mode for [`crate::Server::mtls`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientAuth {
    /// Reject the handshake unless the client presents a certificate that
    /// chains to the configured CA.
    Required,
    /// Verify a presented certificate against the configured CA, but allow
    /// the handshake to complete if the client presents none at all.
    Optional,
}

/// The client's certificate chain from a completed mTLS handshake,
/// DER-encoded, leaf certificate first.
///
/// Inserted into `Request::extensions()` for every request on a connection
/// accepted via [`crate::Server::mtls`] where the client presented a
/// certificate. Absent for plain TLS, non-TLS connections, and for
/// [`ClientAuth::Optional`] connections where no certificate was presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerCertificates(pub Arc<Vec<Vec<u8>>>);

impl PeerCertificates {
    /// The leaf (end-entity) certificate's DER bytes.
    pub fn leaf(&self) -> Option<&[u8]> {
        self.0.first().map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_returns_first_cert() {
        let certs = PeerCertificates(Arc::new(vec![vec![1, 2, 3], vec![4, 5, 6]]));
        assert_eq!(certs.leaf(), Some([1, 2, 3].as_slice()));
    }

    #[test]
    fn leaf_returns_none_when_empty() {
        let certs = PeerCertificates(Arc::new(vec![]));
        assert_eq!(certs.leaf(), None);
    }
}
