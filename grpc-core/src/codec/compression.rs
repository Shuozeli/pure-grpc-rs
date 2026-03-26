use http::header::HeaderValue;
use http::HeaderMap;

/// Supported compression encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompressionEncoding {
    #[cfg(feature = "gzip")]
    Gzip,
    #[cfg(feature = "deflate")]
    Deflate,
    #[cfg(feature = "zstd")]
    Zstd,
}

impl CompressionEncoding {
    /// All supported encodings.
    pub const ENCODINGS: &'static [CompressionEncoding] = &[
        #[cfg(feature = "gzip")]
        CompressionEncoding::Gzip,
        #[cfg(feature = "deflate")]
        CompressionEncoding::Deflate,
        #[cfg(feature = "zstd")]
        CompressionEncoding::Zstd,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            #[cfg(feature = "gzip")]
            CompressionEncoding::Gzip => "gzip",
            #[cfg(feature = "deflate")]
            CompressionEncoding::Deflate => "deflate",
            #[cfg(feature = "zstd")]
            CompressionEncoding::Zstd => "zstd",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            #[cfg(feature = "gzip")]
            "gzip" => Some(CompressionEncoding::Gzip),
            #[cfg(feature = "deflate")]
            "deflate" => Some(CompressionEncoding::Deflate),
            #[cfg(feature = "zstd")]
            "zstd" => Some(CompressionEncoding::Zstd),
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

        let encoding_str = val
            .to_str()
            .map_err(|_| crate::Status::internal("grpc-encoding header is not valid UTF-8"))?;

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
    #[cfg(feature = "deflate")]
    deflate: bool,
    #[cfg(feature = "zstd")]
    zstd: bool,
}

impl EnabledCompressionEncodings {
    pub fn enable(&mut self, encoding: CompressionEncoding) {
        match encoding {
            #[cfg(feature = "gzip")]
            CompressionEncoding::Gzip => self.gzip = true,
            #[cfg(feature = "deflate")]
            CompressionEncoding::Deflate => self.deflate = true,
            #[cfg(feature = "zstd")]
            CompressionEncoding::Zstd => self.zstd = true,
        }
    }

    pub fn is_enabled(self, encoding: CompressionEncoding) -> bool {
        match encoding {
            #[cfg(feature = "gzip")]
            CompressionEncoding::Gzip => self.gzip,
            #[cfg(feature = "deflate")]
            CompressionEncoding::Deflate => self.deflate,
            #[cfg(feature = "zstd")]
            CompressionEncoding::Zstd => self.zstd,
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
            // Safe: all encoding strings are ASCII constants
            Some(
                HeaderValue::from_str(&encodings.join(","))
                    .expect("encoding names are valid ASCII"),
            )
        }
    }
}

pub const ENCODING_HEADER: &str = "grpc-encoding";
pub const ACCEPT_ENCODING_HEADER: &str = "grpc-accept-encoding";

/// Compress data using the given encoding.
///
/// When no compression features are enabled, `CompressionEncoding` is an uninhabited
/// enum, so this function compiles but can never be called.
pub fn compress(
    encoding: CompressionEncoding,
    src: &[u8],
    dst: &mut bytes::BytesMut,
) -> Result<(), std::io::Error> {
    // Suppress unused-variable warnings when no compression features are enabled.
    let _ = (&src, &dst);
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
        #[cfg(feature = "deflate")]
        CompressionEncoding::Deflate => {
            use flate2::write::DeflateEncoder;
            use std::io::Write;

            let mut encoder = DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(src)?;
            let compressed = encoder.finish()?;
            dst.extend_from_slice(&compressed);
            Ok(())
        }
        #[cfg(feature = "zstd")]
        CompressionEncoding::Zstd => {
            let compressed = zstd::encode_all(src, 0)?;
            dst.extend_from_slice(&compressed);
            Ok(())
        }
    }
}

/// Decompress data using the given encoding.
///
/// When no compression features are enabled, `CompressionEncoding` is an uninhabited
/// enum, so this function compiles but can never be called.
pub fn decompress(
    encoding: CompressionEncoding,
    src: &[u8],
    dst: &mut bytes::BytesMut,
) -> Result<(), std::io::Error> {
    let _ = (&src, &dst);
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
        #[cfg(feature = "deflate")]
        CompressionEncoding::Deflate => {
            use flate2::read::DeflateDecoder;
            use std::io::Read;

            let mut decoder = DeflateDecoder::new(src);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            dst.extend_from_slice(&decompressed);
            Ok(())
        }
        #[cfg(feature = "zstd")]
        CompressionEncoding::Zstd => {
            let decompressed = zstd::decode_all(src)?;
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

    #[cfg(feature = "deflate")]
    mod deflate_tests {
        use super::super::*;
        use bytes::BytesMut;

        #[test]
        fn compress_decompress_roundtrip() {
            let data = b"hello world, this is a test of deflate compression!";
            let mut compressed = BytesMut::new();
            compress(CompressionEncoding::Deflate, data, &mut compressed).unwrap();

            assert!(!compressed.is_empty());
            assert_ne!(&compressed[..], data);

            let mut decompressed = BytesMut::new();
            decompress(CompressionEncoding::Deflate, &compressed, &mut decompressed).unwrap();

            assert_eq!(&decompressed[..], data);
        }

        #[test]
        fn encoding_from_str() {
            assert_eq!(
                CompressionEncoding::parse("deflate"),
                Some(CompressionEncoding::Deflate)
            );
        }

        #[test]
        fn enabled_encodings() {
            let mut enabled = EnabledCompressionEncodings::default();
            assert!(!enabled.is_enabled(CompressionEncoding::Deflate));
            enabled.enable(CompressionEncoding::Deflate);
            assert!(enabled.is_enabled(CompressionEncoding::Deflate));
        }

        #[test]
        fn accept_encoding_header_value() {
            let mut enabled = EnabledCompressionEncodings::default();
            enabled.enable(CompressionEncoding::Deflate);
            let val = enabled.into_accept_encoding_header_value().unwrap();
            assert_eq!(val, "deflate");
        }
    }

    #[cfg(feature = "zstd")]
    mod zstd_tests {
        use super::super::*;
        use bytes::BytesMut;

        #[test]
        fn compress_decompress_roundtrip() {
            let data = b"hello world, this is a test of zstd compression!";
            let mut compressed = BytesMut::new();
            compress(CompressionEncoding::Zstd, data, &mut compressed).unwrap();

            assert!(!compressed.is_empty());
            assert_ne!(&compressed[..], data);

            let mut decompressed = BytesMut::new();
            decompress(CompressionEncoding::Zstd, &compressed, &mut decompressed).unwrap();

            assert_eq!(&decompressed[..], data);
        }

        #[test]
        fn encoding_from_str() {
            assert_eq!(
                CompressionEncoding::parse("zstd"),
                Some(CompressionEncoding::Zstd)
            );
        }

        #[test]
        fn enabled_encodings() {
            let mut enabled = EnabledCompressionEncodings::default();
            assert!(!enabled.is_enabled(CompressionEncoding::Zstd));
            enabled.enable(CompressionEncoding::Zstd);
            assert!(enabled.is_enabled(CompressionEncoding::Zstd));
        }

        #[test]
        fn accept_encoding_header_value() {
            let mut enabled = EnabledCompressionEncodings::default();
            enabled.enable(CompressionEncoding::Zstd);
            let val = enabled.into_accept_encoding_header_value().unwrap();
            assert_eq!(val, "zstd");
        }
    }

    #[cfg(feature = "gzip")]
    mod header_negotiation_tests {
        use super::super::*;
        use http::HeaderMap;

        #[test]
        fn from_encoding_header_rejects_unsupported() {
            let mut headers = HeaderMap::new();
            headers.insert("grpc-encoding", "gzip".parse().unwrap());
            let accepted = EnabledCompressionEncodings::default();
            let result = CompressionEncoding::from_encoding_header(&headers, accepted);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.code(), crate::Code::Unimplemented);
            assert!(err.message().contains("not supported"));
        }

        #[test]
        fn from_encoding_header_unknown_encoding() {
            let mut headers = HeaderMap::new();
            headers.insert("grpc-encoding", "brotli".parse().unwrap());
            let accepted = EnabledCompressionEncodings::default();
            let result = CompressionEncoding::from_encoding_header(&headers, accepted);
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .message()
                .contains("unknown compression"));
        }

        #[test]
        fn from_encoding_header_identity_returns_none() {
            let mut headers = HeaderMap::new();
            headers.insert("grpc-encoding", "identity".parse().unwrap());
            let accepted = EnabledCompressionEncodings::default();
            let result = CompressionEncoding::from_encoding_header(&headers, accepted).unwrap();
            assert!(result.is_none());
        }

        #[test]
        fn from_encoding_header_missing_returns_none() {
            let headers = HeaderMap::new();
            let accepted = EnabledCompressionEncodings::default();
            let result = CompressionEncoding::from_encoding_header(&headers, accepted).unwrap();
            assert!(result.is_none());
        }

        #[test]
        fn from_accept_encoding_picks_first_enabled() {
            let mut headers = HeaderMap::new();
            headers.insert("grpc-accept-encoding", "deflate,gzip".parse().unwrap());

            let mut enabled = EnabledCompressionEncodings::default();
            enabled.enable(CompressionEncoding::Gzip);

            let result = CompressionEncoding::from_accept_encoding_header(&headers, enabled);
            assert_eq!(result, Some(CompressionEncoding::Gzip));
        }

        #[test]
        fn from_accept_encoding_no_match() {
            let mut headers = HeaderMap::new();
            headers.insert("grpc-accept-encoding", "brotli,snappy".parse().unwrap());

            let mut enabled = EnabledCompressionEncodings::default();
            enabled.enable(CompressionEncoding::Gzip);

            let result = CompressionEncoding::from_accept_encoding_header(&headers, enabled);
            assert!(result.is_none());
        }
    }
}
