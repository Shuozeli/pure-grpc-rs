use crate::metadata::MetadataMap;
use base64::Engine as _;
use bytes::Bytes;
use http::header::{HeaderMap, HeaderValue};
use http::HeaderName;
use percent_encoding::{percent_decode, percent_encode, AsciiSet, CONTROLS};
use std::{error::Error, fmt, sync::Arc};

const ENCODING_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'?')
    .add(b'{')
    .add(b'}');

/// Generates `Code` enum, `from_i32`, `from_bytes`, `description`, `to_header_value`,
/// `Display`, `From<i32>`, `From<Code> for i32`, and `Status` named constructors
/// from a single definition table.
macro_rules! define_codes {
    (
        $(
            $variant:ident = $num:literal, $str_val:literal, $snake:ident, $desc:literal;
        )*
    ) => {
        /// gRPC status codes.
        ///
        /// These variants match the [gRPC status codes](https://github.com/grpc/grpc/blob/master/doc/statuscodes.md).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Code {
            $( $variant = $num, )*
        }

        impl Code {
            pub const fn from_i32(i: i32) -> Code {
                match i {
                    $( $num => Code::$variant, )*
                    _ => Code::Unknown,
                }
            }

            pub fn from_bytes(bytes: &[u8]) -> Code {
                match bytes {
                    $( $str_val => Code::$variant, )*
                    _ => Code::Unknown,
                }
            }

            pub fn description(&self) -> &'static str {
                match self {
                    $( Code::$variant => $desc, )*
                }
            }

            fn to_header_value(self) -> HeaderValue {
                match self {
                    $( Code::$variant => HeaderValue::from_static(
                        ::std::stringify!($num)
                    ), )*
                }
            }
        }

        impl fmt::Display for Code {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self.description(), f)
            }
        }

        impl From<i32> for Code {
            fn from(i: i32) -> Self {
                Code::from_i32(i)
            }
        }

        impl From<Code> for i32 {
            fn from(code: Code) -> i32 {
                code as i32
            }
        }

        impl Status {
            $(
                pub fn $snake(message: impl Into<String>) -> Status {
                    Status::new(Code::$variant, message)
                }
            )*
        }
    };
}

define_codes! {
    Ok = 0, b"0", ok, "The operation completed successfully";
    Cancelled = 1, b"1", cancelled, "The operation was cancelled";
    Unknown = 2, b"2", unknown, "Unknown error";
    InvalidArgument = 3, b"3", invalid_argument, "Client specified an invalid argument";
    DeadlineExceeded = 4, b"4", deadline_exceeded, "Deadline expired before operation could complete";
    NotFound = 5, b"5", not_found, "Some requested entity was not found";
    AlreadyExists = 6, b"6", already_exists, "Some entity that we attempted to create already exists";
    PermissionDenied = 7, b"7", permission_denied, "The caller does not have permission to execute the specified operation";
    ResourceExhausted = 8, b"8", resource_exhausted, "Some resource has been exhausted";
    FailedPrecondition = 9, b"9", failed_precondition, "The system is not in a state required for the operation's execution";
    Aborted = 10, b"10", aborted, "The operation was aborted";
    OutOfRange = 11, b"11", out_of_range, "Operation was attempted past the valid range";
    Unimplemented = 12, b"12", unimplemented, "Operation is not implemented or not supported";
    Internal = 13, b"13", internal, "Internal error";
    Unavailable = 14, b"14", unavailable, "The service is currently unavailable";
    DataLoss = 15, b"15", data_loss, "Unrecoverable data loss or corruption";
    Unauthenticated = 16, b"16", unauthenticated, "The request does not have valid authentication credentials";
}

// --- Status ---

/// A gRPC status describing the result of an RPC call.
///
/// Boxed inner for size optimization (keeps `Result<T, Status>` small).
#[derive(Clone)]
pub struct Status(Box<StatusInner>);

#[derive(Clone)]
struct StatusInner {
    code: Code,
    message: String,
    details: Bytes,
    metadata: MetadataMap,
    source: Option<Arc<dyn Error + Send + Sync + 'static>>,
}

impl Status {
    pub const GRPC_STATUS: HeaderName = HeaderName::from_static("grpc-status");
    pub const GRPC_MESSAGE: HeaderName = HeaderName::from_static("grpc-message");
    pub const GRPC_STATUS_DETAILS: HeaderName = HeaderName::from_static("grpc-status-details-bin");

    pub fn new(code: Code, message: impl Into<String>) -> Status {
        Status(Box::new(StatusInner {
            code,
            message: message.into(),
            details: Bytes::new(),
            metadata: MetadataMap::new(),
            source: None,
        }))
    }

    pub fn with_details(code: Code, message: impl Into<String>, details: Bytes) -> Status {
        Self::with_details_and_metadata(code, message, details, MetadataMap::new())
    }

    pub fn with_metadata(code: Code, message: impl Into<String>, metadata: MetadataMap) -> Status {
        Self::with_details_and_metadata(code, message, Bytes::new(), metadata)
    }

    pub fn with_details_and_metadata(
        code: Code,
        message: impl Into<String>,
        details: Bytes,
        metadata: MetadataMap,
    ) -> Status {
        Status(Box::new(StatusInner {
            code,
            message: message.into(),
            details,
            metadata,
            source: None,
        }))
    }

    pub fn code(&self) -> Code {
        self.0.code
    }

    pub fn message(&self) -> &str {
        &self.0.message
    }

    pub fn details(&self) -> &[u8] {
        &self.0.details
    }

    pub fn metadata(&self) -> &MetadataMap {
        &self.0.metadata
    }

    pub fn metadata_mut(&mut self) -> &mut MetadataMap {
        &mut self.0.metadata
    }

    pub fn set_source(&mut self, source: Arc<dyn Error + Send + Sync + 'static>) -> &mut Status {
        self.0.source = Some(source);
        self
    }

    /// Create a `Status` from a boxed error, walking the source chain.
    ///
    /// Inspects the error and its sources for recognizable types:
    /// - `Status` — returned directly
    /// - `io::Error` — mapped via `From<io::Error>`
    /// - Connection errors — mapped to `Unavailable`
    /// - Timeout errors — mapped to `DeadlineExceeded`
    /// - Everything else — `Unknown`
    pub fn from_error(err: Box<dyn Error + Send + Sync + 'static>) -> Status {
        // Try direct downcast to Status
        match err.downcast::<Status>() {
            Ok(status) => *status,
            Err(err) => {
                // Walk the source chain for recognizable errors
                if let Some(status) = find_status_in_source_chain(&*err) {
                    return status;
                }

                let mut status = Status::new(Code::Unknown, err.to_string());
                status.0.source = Some(err.into());
                status
            }
        }
    }

    pub(crate) fn map_error<E: Into<Box<dyn Error + Send + Sync>>>(err: E) -> Status {
        Status::from_error(err.into())
    }

    /// Extract a `Status` from HTTP trailing headers.
    pub fn from_header_map(header_map: &HeaderMap) -> Option<Status> {
        let code = Code::from_bytes(header_map.get(Self::GRPC_STATUS)?.as_ref());

        let error_message = match header_map.get(Self::GRPC_MESSAGE) {
            Some(header) => percent_decode(header.as_bytes())
                .decode_utf8()
                .map(|cow| cow.to_string()),
            None => Ok(String::new()),
        };

        let details = match header_map.get(Self::GRPC_STATUS_DETAILS) {
            Some(header) => match base64_engine().decode(header.as_bytes()) {
                Ok(bytes) => bytes.into(),
                Err(_) => {
                    // Corrupted status details — log context in the message
                    return Some(Status::new(
                        Code::Internal,
                        "invalid base64 in grpc-status-details-bin header",
                    ));
                }
            },
            None => Bytes::new(),
        };

        let other_headers = {
            let mut hm = HeaderMap::with_capacity(header_map.len());
            for (key, value) in header_map.iter() {
                if key != Self::GRPC_STATUS
                    && key != Self::GRPC_MESSAGE
                    && key != Self::GRPC_STATUS_DETAILS
                {
                    hm.insert(key.clone(), value.clone());
                }
            }
            hm
        };

        let (code, message) = match error_message {
            Ok(message) => (code, message),
            Err(e) => {
                // Keep the original gRPC status code; only the message is degraded.
                let msg = format!("Error deserializing status message header: {e}");
                (code, msg)
            }
        };

        Some(Status::with_details_and_metadata(
            code,
            message,
            details,
            MetadataMap::from_headers(other_headers),
        ))
    }

    /// Serialize this status into HTTP headers.
    pub fn add_header(&self, header_map: &mut HeaderMap) -> Result<(), Self> {
        self.0.metadata.extend_sanitized_into(header_map);
        header_map.insert(Self::GRPC_STATUS, self.0.code.to_header_value());

        if !self.0.message.is_empty() {
            let mut buf = bytes::BytesMut::new();
            for chunk in percent_encode(self.message().as_bytes(), ENCODING_SET) {
                buf.extend_from_slice(chunk.as_bytes());
            }
            let encoded = buf.freeze();
            header_map.insert(
                Self::GRPC_MESSAGE,
                HeaderValue::from_maybe_shared(encoded)
                    .map_err(|_| Status::internal("invalid grpc-message header value"))?,
            );
        }

        if !self.0.details.is_empty() {
            let encoded = base64_no_pad_engine().encode(&self.0.details[..]);
            header_map.insert(
                Self::GRPC_STATUS_DETAILS,
                HeaderValue::from_maybe_shared(encoded)
                    .map_err(|_| Status::internal("invalid grpc-status-details-bin value"))?,
            );
        }

        Ok(())
    }

    pub(crate) fn to_header_map(&self) -> Result<HeaderMap, Self> {
        let mut header_map = HeaderMap::with_capacity(3 + self.0.metadata.len());
        self.add_header(&mut header_map)?;
        Ok(header_map)
    }

    /// Build an `http::Response` from this `Status` (trailers-only response).
    ///
    /// If the status message contains bytes that cannot be encoded into a valid
    /// header value, the message is dropped and only the status code is sent.
    pub fn into_http<B: Default>(self) -> http::Response<B> {
        let mut response = http::Response::new(B::default());
        response.headers_mut().insert(
            http::header::CONTENT_TYPE,
            crate::metadata::GRPC_CONTENT_TYPE,
        );
        if let Err(fallback) = self.add_header(response.headers_mut()) {
            // The message couldn't be encoded; send just the code.
            response
                .headers_mut()
                .insert(Self::GRPC_STATUS, fallback.code().to_header_value());
        }
        response
    }
}

impl fmt::Debug for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut builder = f.debug_struct("Status");
        builder.field("code", &self.0.code);
        if !self.0.message.is_empty() {
            builder.field("message", &self.0.message);
        }
        if !self.0.details.is_empty() {
            builder.field("details", &self.0.details);
        }
        builder.field("source", &self.0.source);
        builder.finish()
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "code: '{}'", self.code())?;
        if !self.message().is_empty() {
            write!(f, ", message: {:?}", self.message())?;
        }
        if let Some(source) = self.source() {
            write!(f, ", source: {source:?}")?;
        }
        Ok(())
    }
}

impl Error for Status {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source.as_ref().map(|err| (&**err) as _)
    }
}

impl From<std::io::Error> for Status {
    fn from(err: std::io::Error) -> Self {
        use std::io::ErrorKind;
        let code = match err.kind() {
            ErrorKind::BrokenPipe
            | ErrorKind::WouldBlock
            | ErrorKind::WriteZero
            | ErrorKind::Interrupted => Code::Internal,
            ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::NotConnected
            | ErrorKind::AddrInUse
            | ErrorKind::AddrNotAvailable => Code::Unavailable,
            ErrorKind::AlreadyExists => Code::AlreadyExists,
            ErrorKind::ConnectionAborted => Code::Aborted,
            ErrorKind::InvalidData => Code::DataLoss,
            ErrorKind::InvalidInput => Code::InvalidArgument,
            ErrorKind::NotFound => Code::NotFound,
            ErrorKind::PermissionDenied => Code::PermissionDenied,
            ErrorKind::TimedOut => Code::DeadlineExceeded,
            ErrorKind::UnexpectedEof => Code::OutOfRange,
            _ => Code::Unknown,
        };
        Status::new(code, err.to_string())
    }
}

/// Walk the error source chain looking for recognizable error types.
fn find_status_in_source_chain(err: &(dyn Error + 'static)) -> Option<Status> {
    let mut source = Some(err);

    while let Some(err) = source {
        // Check for Status
        if let Some(status) = err.downcast_ref::<Status>() {
            return Some(Status::new(status.code(), status.message()));
        }

        // Check for io::Error
        if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
            return Some(Status::from(std::io::Error::new(
                io_err.kind(),
                io_err.to_string(),
            )));
        }

        source = err.source();
    }

    None
}

/// Infer gRPC status from trailers or HTTP status code.
pub(crate) fn infer_grpc_status(
    trailers: Option<&HeaderMap>,
    status_code: http::StatusCode,
) -> Result<(), Option<Status>> {
    if let Some(trailers) = trailers {
        if let Some(status) = Status::from_header_map(trailers) {
            if status.code() == Code::Ok {
                return Ok(());
            } else {
                return Err(Some(status));
            }
        }
    }

    let code = match status_code {
        http::StatusCode::BAD_REQUEST => Code::Internal,
        http::StatusCode::UNAUTHORIZED => Code::Unauthenticated,
        http::StatusCode::FORBIDDEN => Code::PermissionDenied,
        http::StatusCode::NOT_FOUND => Code::NotFound,
        http::StatusCode::TOO_MANY_REQUESTS
        | http::StatusCode::BAD_GATEWAY
        | http::StatusCode::SERVICE_UNAVAILABLE
        | http::StatusCode::GATEWAY_TIMEOUT => Code::Unavailable,
        http::StatusCode::OK => return Err(None),
        _ => Code::Unknown,
    };

    let msg = format!(
        "grpc-status header missing, mapped from HTTP status code {}",
        status_code.as_u16(),
    );
    Err(Some(Status::new(code, msg)))
}

// --- Base64 engines ---

fn base64_engine() -> base64::engine::GeneralPurpose {
    use base64::{alphabet, engine::general_purpose::GeneralPurposeConfig};
    base64::engine::GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_encode_padding(true)
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    )
}

fn base64_no_pad_engine() -> base64::engine::GeneralPurpose {
    use base64::{alphabet, engine::general_purpose::GeneralPurposeConfig};
    base64::engine::GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_encode_padding(false)
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_from_i32_covers_all_variants() {
        for i in 0..=16 {
            let code = Code::from_i32(i);
            assert_eq!(i, code as i32);
        }
        assert_eq!(Code::from_i32(-1), Code::Unknown);
        assert_eq!(Code::from_i32(999), Code::Unknown);
    }

    #[test]
    fn code_from_bytes_parses_correctly() {
        assert_eq!(Code::from_bytes(b"0"), Code::Ok);
        assert_eq!(Code::from_bytes(b"1"), Code::Cancelled);
        assert_eq!(Code::from_bytes(b"16"), Code::Unauthenticated);
        assert_eq!(Code::from_bytes(b"99"), Code::Unknown);
        assert_eq!(Code::from_bytes(b"abc"), Code::Unknown);
    }

    #[test]
    fn named_constructors_return_correct_code() {
        assert_eq!(Status::ok("").code(), Code::Ok);
        assert_eq!(Status::cancelled("").code(), Code::Cancelled);
        assert_eq!(Status::unknown("").code(), Code::Unknown);
        assert_eq!(Status::invalid_argument("").code(), Code::InvalidArgument);
        assert_eq!(Status::not_found("").code(), Code::NotFound);
        assert_eq!(Status::internal("").code(), Code::Internal);
        assert_eq!(Status::unavailable("").code(), Code::Unavailable);
        assert_eq!(Status::unimplemented("").code(), Code::Unimplemented);
        assert_eq!(Status::unauthenticated("").code(), Code::Unauthenticated);
    }

    #[test]
    fn status_message_preserved() {
        let status = Status::new(Code::NotFound, "the thing");
        assert_eq!(status.message(), "the thing");
    }

    #[test]
    fn status_details_roundtrip_through_headers() {
        let details: &[u8] = &[0, 2, 3];
        let status = Status::with_details(Code::Unavailable, "msg", Bytes::from_static(details));

        let header_map = status.to_header_map().unwrap();
        let decoded = Status::from_header_map(&header_map).unwrap();

        assert_eq!(decoded.code(), Code::Unavailable);
        assert_eq!(decoded.message(), "msg");
        assert_eq!(decoded.details(), details);
    }

    #[test]
    fn status_header_roundtrip_with_special_chars() {
        let status = Status::new(Code::Internal, "hello world {test}");
        let header_map = status.to_header_map().unwrap();
        let decoded = Status::from_header_map(&header_map).unwrap();

        assert_eq!(decoded.code(), Code::Internal);
        assert_eq!(decoded.message(), "hello world {test}");
    }

    #[test]
    fn from_io_error_maps_correctly() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let status: Status = err.into();
        assert_eq!(status.code(), Code::NotFound);

        let err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let status: Status = err.into();
        assert_eq!(status.code(), Code::Unavailable);

        let err = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        let status: Status = err.into();
        assert_eq!(status.code(), Code::DeadlineExceeded);
    }

    #[test]
    fn from_error_preserves_status() {
        let orig = Status::new(Code::OutOfRange, "weeaboo");
        let found = Status::from_error(Box::new(orig));
        assert_eq!(found.code(), Code::OutOfRange);
        assert_eq!(found.message(), "weeaboo");
    }

    #[test]
    fn from_error_unknown_for_generic_error() {
        let err: crate::BoxError = "peek-a-boo".into();
        let found = Status::from_error(err);
        assert_eq!(found.code(), Code::Unknown);
        assert_eq!(found.message(), "peek-a-boo");
    }

    #[test]
    fn into_http_produces_valid_response() {
        let status = Status::new(Code::NotFound, "missing");
        let resp: http::Response<()> = status.into_http();
        let headers = resp.headers();
        assert_eq!(headers.get("grpc-status").unwrap(), "5");
        assert_eq!(headers.get("content-type").unwrap(), "application/grpc");
    }

    #[test]
    fn infer_grpc_status_from_trailers() {
        let mut trailers = HeaderMap::new();
        trailers.insert("grpc-status", HeaderValue::from_static("0"));
        assert!(infer_grpc_status(Some(&trailers), http::StatusCode::OK).is_ok());

        trailers.insert("grpc-status", HeaderValue::from_static("5"));
        let err = infer_grpc_status(Some(&trailers), http::StatusCode::OK).unwrap_err();
        assert_eq!(err.unwrap().code(), Code::NotFound);
    }

    #[test]
    fn infer_grpc_status_from_http_status() {
        let err = infer_grpc_status(None, http::StatusCode::UNAUTHORIZED).unwrap_err();
        assert_eq!(err.unwrap().code(), Code::Unauthenticated);

        let err = infer_grpc_status(None, http::StatusCode::NOT_FOUND).unwrap_err();
        assert_eq!(err.unwrap().code(), Code::NotFound);
    }
}
