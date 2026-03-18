use http::header::HeaderValue;
use http::HeaderMap;

/// Supported compression encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompressionEncoding {
    #[cfg(feature = "gzip")]
    Gzip,
}

impl CompressionEncoding {
    /// All supported encodings.
    pub const ENCODINGS: &'static [CompressionEncoding] = &[
        #[cfg(feature = "gzip")]
        CompressionEncoding::Gzip,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            #[cfg(feature = "gzip")]
            CompressionEncoding::Gzip => "gzip",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            #[cfg(feature = "gzip")]
            "gzip" => Some(CompressionEncoding::Gzip),
            _ => None,
        }
    }

    pub fn into_header_value(self) -> HeaderValue {
        HeaderValue::from_static(self.as_str())
    }

    /// Parse `grpc-encoding` header from a request/response.
    pub fn from_encoding_header(
        headers: &HeaderMap,
        accepted: EnabledCompressionEncodings,
    ) -> Result<Option<Self>, crate::Status> {
        let Some(val) = headers.get(ENCODING_HEADER) else {
            return Ok(None);
        };

        let encoding_str = val.to_str().unwrap_or("");

        if encoding_str == "identity" {
            return Ok(None);
        }

        match Self::parse(encoding_str) {
            Some(encoding) if accepted.is_enabled(encoding) => Ok(Some(encoding)),
            Some(encoding) => Err(crate::Status::unimplemented(format!(
                "compression `{}` is not supported",
                encoding.as_str()
            ))),
            None => Err(crate::Status::unimplemented(format!(
                "unknown compression encoding `{encoding_str}`"
            ))),
        }
    }

    /// Parse `grpc-accept-encoding` to find a mutually supported encoding.
    pub fn from_accept_encoding_header(
        headers: &HeaderMap,
        send_encodings: EnabledCompressionEncodings,
    ) -> Option<Self> {
        let val = headers.get(ACCEPT_ENCODING_HEADER)?;
        let accept_str = val.to_str().ok()?;

        for part in accept_str.split(',') {
            let encoding_str = part.trim();
            if let Some(encoding) = Self::parse(encoding_str) {
                if send_encodings.is_enabled(encoding) {
                    return Some(encoding);
                }
            }
        }

        None
    }
}

/// Set of enabled compression encodings.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnabledCompressionEncodings {
    #[cfg(feature = "gzip")]
    gzip: bool,
}

impl EnabledCompressionEncodings {
    pub fn enable(&mut self, encoding: CompressionEncoding) {
        match encoding {
            #[cfg(feature = "gzip")]
            CompressionEncoding::Gzip => self.gzip = true,
        }
    }

    pub fn is_enabled(self, encoding: CompressionEncoding) -> bool {
        match encoding {
            #[cfg(feature = "gzip")]
            CompressionEncoding::Gzip => self.gzip,
        }
    }

    /// Build the `grpc-accept-encoding` header value.
    pub fn into_accept_encoding_header_value(self) -> Option<HeaderValue> {
        let mut encodings = Vec::new();
        for &enc in CompressionEncoding::ENCODINGS {
            if self.is_enabled(enc) {
                encodings.push(enc.as_str());
            }
        }
        if encodings.is_empty() {
            None
        } else {
            Some(HeaderValue::from_str(&encodings.join(",")).unwrap())
        }
    }
}

pub const ENCODING_HEADER: &str = "grpc-encoding";
pub const ACCEPT_ENCODING_HEADER: &str = "grpc-accept-encoding";

/// Compress data using the given encoding.
pub fn compress(
    encoding: CompressionEncoding,
    src: &[u8],
    dst: &mut bytes::BytesMut,
) -> Result<(), std::io::Error> {
    let _ = (&encoding, src, &dst);
    match encoding {
        #[cfg(feature = "gzip")]
        CompressionEncoding::Gzip => {
            use flate2::write::GzEncoder;
            use std::io::Write;

            let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(src)?;
            let compressed = encoder.finish()?;
            dst.extend_from_slice(&compressed);
            Ok(())
        }
    }
}

/// Decompress data using the given encoding.
pub fn decompress(
    encoding: CompressionEncoding,
    src: &[u8],
    dst: &mut bytes::BytesMut,
) -> Result<(), std::io::Error> {
    let _ = (&encoding, src, &dst);
    match encoding {
        #[cfg(feature = "gzip")]
        CompressionEncoding::Gzip => {
            use flate2::read::GzDecoder;
            use std::io::Read;

            let mut decoder = GzDecoder::new(src);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            dst.extend_from_slice(&decompressed);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "gzip")]
    mod gzip_tests {
        use super::super::*;
        use bytes::BytesMut;

        #[test]
        fn compress_decompress_roundtrip() {
            let data = b"hello world, this is a test of gzip compression!";
            let mut compressed = BytesMut::new();
            compress(CompressionEncoding::Gzip, data, &mut compressed).unwrap();

            assert!(!compressed.is_empty());
            assert_ne!(&compressed[..], data);

            let mut decompressed = BytesMut::new();
            decompress(CompressionEncoding::Gzip, &compressed, &mut decompressed).unwrap();

            assert_eq!(&decompressed[..], data);
        }

        #[test]
        fn encoding_from_str() {
            assert_eq!(
                CompressionEncoding::parse("gzip"),
                Some(CompressionEncoding::Gzip)
            );
            assert_eq!(CompressionEncoding::parse("unknown"), None);
            assert_eq!(CompressionEncoding::parse("identity"), None);
        }

        #[test]
        fn enabled_encodings() {
            let mut enabled = EnabledCompressionEncodings::default();
            assert!(!enabled.is_enabled(CompressionEncoding::Gzip));
            enabled.enable(CompressionEncoding::Gzip);
            assert!(enabled.is_enabled(CompressionEncoding::Gzip));
        }

        #[test]
        fn accept_encoding_header_value() {
            let mut enabled = EnabledCompressionEncodings::default();
            assert!(enabled.into_accept_encoding_header_value().is_none());

            enabled.enable(CompressionEncoding::Gzip);
            let val = enabled.into_accept_encoding_header_value().unwrap();
            assert_eq!(val, "gzip");
        }
    }
}
