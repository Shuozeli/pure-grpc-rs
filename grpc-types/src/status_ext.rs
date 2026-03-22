//! Extension trait for attaching/extracting rich error details on `grpc_core::Status`.

use bytes::Bytes;
use grpc_core::status::{Code, Status};
use prost::{DecodeError, Message};

use crate::any_ext::{decode_any, ErrorDetail, IntoAny};
use crate::google::rpc::*;

/// Extension trait for `grpc_core::Status` providing rich error detail support.
///
/// Enables constructing a `Status` with typed error details (google.rpc.*)
/// and extracting them back.
pub trait StatusExt {
    /// Construct a `Status` with rich error details.
    ///
    /// Details are packed into `prost_types::Any` messages, wrapped in a
    /// `google.rpc.Status` protobuf message, and stored in the `details` field.
    fn with_error_details(
        code: Code,
        message: impl Into<String>,
        details: Vec<ErrorDetail>,
    ) -> Status;

    /// Decode the `details` bytes into a list of typed `ErrorDetail` values.
    ///
    /// Returns an empty vec if no details are present.
    /// Returns a `DecodeError` if the bytes are not a valid `google.rpc.Status` message.
    fn error_details(&self) -> Result<Vec<ErrorDetail>, DecodeError>;

    /// Extract the first `ErrorInfo` detail, if present.
    fn get_details_error_info(&self) -> Option<ErrorInfo>;
    /// Extract the first `RetryInfo` detail, if present.
    fn get_details_retry_info(&self) -> Option<RetryInfo>;
    /// Extract the first `DebugInfo` detail, if present.
    fn get_details_debug_info(&self) -> Option<DebugInfo>;
    /// Extract the first `QuotaFailure` detail, if present.
    fn get_details_quota_failure(&self) -> Option<QuotaFailure>;
    /// Extract the first `PreconditionFailure` detail, if present.
    fn get_details_precondition_failure(&self) -> Option<PreconditionFailure>;
    /// Extract the first `BadRequest` detail, if present.
    fn get_details_bad_request(&self) -> Option<BadRequest>;
    /// Extract the first `RequestInfo` detail, if present.
    fn get_details_request_info(&self) -> Option<RequestInfo>;
    /// Extract the first `ResourceInfo` detail, if present.
    fn get_details_resource_info(&self) -> Option<ResourceInfo>;
    /// Extract the first `Help` detail, if present.
    fn get_details_help(&self) -> Option<Help>;
    /// Extract the first `LocalizedMessage` detail, if present.
    fn get_details_localized_message(&self) -> Option<LocalizedMessage>;
}

impl StatusExt for Status {
    fn with_error_details(
        code: Code,
        message: impl Into<String>,
        details: Vec<ErrorDetail>,
    ) -> Status {
        let message = message.into();
        let any_details: Vec<prost_types::Any> = details
            .into_iter()
            .map(|d| match d {
                ErrorDetail::ErrorInfo(v) => v.into_any(),
                ErrorDetail::RetryInfo(v) => v.into_any(),
                ErrorDetail::DebugInfo(v) => v.into_any(),
                ErrorDetail::QuotaFailure(v) => v.into_any(),
                ErrorDetail::PreconditionFailure(v) => v.into_any(),
                ErrorDetail::BadRequest(v) => v.into_any(),
                ErrorDetail::RequestInfo(v) => v.into_any(),
                ErrorDetail::ResourceInfo(v) => v.into_any(),
                ErrorDetail::Help(v) => v.into_any(),
                ErrorDetail::LocalizedMessage(v) => v.into_any(),
                ErrorDetail::Unknown(any) => any,
            })
            .collect();

        let rpc_status = crate::google::rpc::Status {
            code: code as i32,
            message: message.clone(),
            details: any_details,
        };

        let encoded = rpc_status.encode_to_vec();
        Status::with_details(code, message, Bytes::from(encoded))
    }

    fn error_details(&self) -> Result<Vec<ErrorDetail>, DecodeError> {
        let details_bytes = self.details();
        if details_bytes.is_empty() {
            return Ok(vec![]);
        }

        let rpc_status = crate::google::rpc::Status::decode(details_bytes)?;
        rpc_status.details.iter().map(decode_any).collect()
    }

    fn get_details_error_info(&self) -> Option<ErrorInfo> {
        self.error_details()
            .ok()?
            .into_iter()
            .find_map(|d| match d {
                ErrorDetail::ErrorInfo(v) => Some(v),
                _ => None,
            })
    }

    fn get_details_retry_info(&self) -> Option<RetryInfo> {
        self.error_details()
            .ok()?
            .into_iter()
            .find_map(|d| match d {
                ErrorDetail::RetryInfo(v) => Some(v),
                _ => None,
            })
    }

    fn get_details_debug_info(&self) -> Option<DebugInfo> {
        self.error_details()
            .ok()?
            .into_iter()
            .find_map(|d| match d {
                ErrorDetail::DebugInfo(v) => Some(v),
                _ => None,
            })
    }

    fn get_details_quota_failure(&self) -> Option<QuotaFailure> {
        self.error_details()
            .ok()?
            .into_iter()
            .find_map(|d| match d {
                ErrorDetail::QuotaFailure(v) => Some(v),
                _ => None,
            })
    }

    fn get_details_precondition_failure(&self) -> Option<PreconditionFailure> {
        self.error_details()
            .ok()?
            .into_iter()
            .find_map(|d| match d {
                ErrorDetail::PreconditionFailure(v) => Some(v),
                _ => None,
            })
    }

    fn get_details_bad_request(&self) -> Option<BadRequest> {
        self.error_details()
            .ok()?
            .into_iter()
            .find_map(|d| match d {
                ErrorDetail::BadRequest(v) => Some(v),
                _ => None,
            })
    }

    fn get_details_request_info(&self) -> Option<RequestInfo> {
        self.error_details()
            .ok()?
            .into_iter()
            .find_map(|d| match d {
                ErrorDetail::RequestInfo(v) => Some(v),
                _ => None,
            })
    }

    fn get_details_resource_info(&self) -> Option<ResourceInfo> {
        self.error_details()
            .ok()?
            .into_iter()
            .find_map(|d| match d {
                ErrorDetail::ResourceInfo(v) => Some(v),
                _ => None,
            })
    }

    fn get_details_help(&self) -> Option<Help> {
        self.error_details()
            .ok()?
            .into_iter()
            .find_map(|d| match d {
                ErrorDetail::Help(v) => Some(v),
                _ => None,
            })
    }

    fn get_details_localized_message(&self) -> Option<LocalizedMessage> {
        self.error_details()
            .ok()?
            .into_iter()
            .find_map(|d| match d {
                ErrorDetail::LocalizedMessage(v) => Some(v),
                _ => None,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn status_with_error_details_and_extract() {
        let error_info = ErrorInfo {
            reason: "API_DISABLED".into(),
            domain: "googleapis.com".into(),
            metadata: HashMap::from([("service".into(), "pubsub".into())]),
        };
        let bad_request = BadRequest {
            field_violations: vec![bad_request::FieldViolation {
                field: "name".into(),
                description: "must not be empty".into(),
                reason: String::new(),
                localized_message: None,
            }],
        };

        let status = Status::with_error_details(
            Code::InvalidArgument,
            "bad request",
            vec![error_info.clone().into(), bad_request.clone().into()],
        );

        assert_eq!(status.code(), Code::InvalidArgument);
        assert_eq!(status.message(), "bad request");

        let details = status.error_details().unwrap();
        assert_eq!(details.len(), 2);
        assert_eq!(details[0], ErrorDetail::ErrorInfo(error_info));
        assert_eq!(details[1], ErrorDetail::BadRequest(bad_request));
    }

    #[test]
    fn status_empty_details_returns_empty_vec() {
        let status = Status::new(Code::Ok, "success");
        let details = status.error_details().unwrap();
        assert!(details.is_empty());
    }

    #[test]
    fn get_details_specific_type() {
        let retry_info = RetryInfo {
            retry_delay: Some(prost_types::Duration {
                seconds: 30,
                nanos: 0,
            }),
        };
        let error_info = ErrorInfo {
            reason: "RATE_LIMITED".into(),
            domain: "example.com".into(),
            metadata: HashMap::new(),
        };

        let status = Status::with_error_details(
            Code::ResourceExhausted,
            "rate limited",
            vec![error_info.clone().into(), retry_info.into()],
        );

        // Should find RetryInfo
        let found = status.get_details_retry_info().unwrap();
        assert_eq!(found.retry_delay.unwrap().seconds, 30);

        // Should find ErrorInfo
        let found = status.get_details_error_info().unwrap();
        assert_eq!(found.reason, "RATE_LIMITED");

        // Should not find BadRequest
        assert!(status.get_details_bad_request().is_none());
    }

    #[test]
    fn full_header_roundtrip() {
        let error_info = ErrorInfo {
            reason: "QUOTA_EXCEEDED".into(),
            domain: "example.com".into(),
            metadata: HashMap::new(),
        };

        let original = Status::with_error_details(
            Code::ResourceExhausted,
            "quota exceeded",
            vec![error_info.clone().into()],
        );

        // Encode to HTTP response (trailers)
        let http_response: http::Response<()> = original.into_http();
        let headers = http_response.headers();

        // Decode from headers (simulating what the client does)
        let decoded = Status::from_header_map(headers).expect("should decode status from headers");
        assert_eq!(decoded.code(), Code::ResourceExhausted);
        assert_eq!(decoded.message(), "quota exceeded");

        // Extract details from the decoded status
        let details = decoded.error_details().unwrap();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0], ErrorDetail::ErrorInfo(error_info));
    }
}
