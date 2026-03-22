//! Traits for packing/unpacking typed error details into `prost_types::Any`.

use prost::{DecodeError, Message};
use prost_types::Any;

use crate::google::rpc::*;

/// Pack a typed error detail into `prost_types::Any`.
pub trait IntoAny {
    /// The type URL for this message type (e.g., `"type.googleapis.com/google.rpc.ErrorInfo"`).
    const TYPE_URL: &'static str;

    /// Encode this message into an `Any`.
    fn into_any(self) -> Any;
}

/// Unpack a typed error detail from `prost_types::Any`.
pub trait FromAny: Sized {
    /// The type URL for this message type.
    const TYPE_URL: &'static str;

    /// Decode this message from an `Any`.
    fn from_any(any: &Any) -> Result<Self, DecodeError>;
}

/// A decoded error detail, dispatched by type URL.
#[derive(Clone, Debug, PartialEq)]
pub enum ErrorDetail {
    ErrorInfo(ErrorInfo),
    RetryInfo(RetryInfo),
    DebugInfo(DebugInfo),
    QuotaFailure(QuotaFailure),
    PreconditionFailure(PreconditionFailure),
    BadRequest(BadRequest),
    RequestInfo(RequestInfo),
    ResourceInfo(ResourceInfo),
    Help(Help),
    LocalizedMessage(LocalizedMessage),
    /// An unrecognized type URL. The original `Any` is preserved.
    Unknown(Any),
}

/// Decode an `Any` into a known `ErrorDetail` variant, or `Unknown` if the type URL
/// is not recognized.
pub fn decode_any(any: &Any) -> Result<ErrorDetail, DecodeError> {
    match any.type_url.as_str() {
        <ErrorInfo as IntoAny>::TYPE_URL => Ok(ErrorDetail::ErrorInfo(ErrorInfo::from_any(any)?)),
        <RetryInfo as IntoAny>::TYPE_URL => Ok(ErrorDetail::RetryInfo(RetryInfo::from_any(any)?)),
        <DebugInfo as IntoAny>::TYPE_URL => Ok(ErrorDetail::DebugInfo(DebugInfo::from_any(any)?)),
        <QuotaFailure as IntoAny>::TYPE_URL => {
            Ok(ErrorDetail::QuotaFailure(QuotaFailure::from_any(any)?))
        }
        <PreconditionFailure as IntoAny>::TYPE_URL => Ok(ErrorDetail::PreconditionFailure(
            PreconditionFailure::from_any(any)?,
        )),
        <BadRequest as IntoAny>::TYPE_URL => {
            Ok(ErrorDetail::BadRequest(BadRequest::from_any(any)?))
        }
        <RequestInfo as IntoAny>::TYPE_URL => {
            Ok(ErrorDetail::RequestInfo(RequestInfo::from_any(any)?))
        }
        <ResourceInfo as IntoAny>::TYPE_URL => {
            Ok(ErrorDetail::ResourceInfo(ResourceInfo::from_any(any)?))
        }
        <Help as IntoAny>::TYPE_URL => Ok(ErrorDetail::Help(Help::from_any(any)?)),
        <LocalizedMessage as IntoAny>::TYPE_URL => Ok(ErrorDetail::LocalizedMessage(
            LocalizedMessage::from_any(any)?,
        )),
        _ => Ok(ErrorDetail::Unknown(any.clone())),
    }
}

// Macro to reduce boilerplate for IntoAny/FromAny impls.
macro_rules! impl_any_traits {
    ($variant:ident, $type:ty, $url:expr) => {
        impl IntoAny for $type {
            const TYPE_URL: &'static str = $url;

            fn into_any(self) -> Any {
                Any {
                    type_url: $url.to_string(),
                    value: self.encode_to_vec(),
                }
            }
        }

        impl FromAny for $type {
            const TYPE_URL: &'static str = $url;

            fn from_any(any: &Any) -> Result<Self, DecodeError> {
                <$type>::decode(any.value.as_slice())
            }
        }

        impl From<$type> for ErrorDetail {
            fn from(v: $type) -> Self {
                ErrorDetail::$variant(v)
            }
        }
    };
}

impl_any_traits!(
    ErrorInfo,
    ErrorInfo,
    "type.googleapis.com/google.rpc.ErrorInfo"
);
impl_any_traits!(
    RetryInfo,
    RetryInfo,
    "type.googleapis.com/google.rpc.RetryInfo"
);
impl_any_traits!(
    DebugInfo,
    DebugInfo,
    "type.googleapis.com/google.rpc.DebugInfo"
);
impl_any_traits!(
    QuotaFailure,
    QuotaFailure,
    "type.googleapis.com/google.rpc.QuotaFailure"
);
impl_any_traits!(
    PreconditionFailure,
    PreconditionFailure,
    "type.googleapis.com/google.rpc.PreconditionFailure"
);
impl_any_traits!(
    BadRequest,
    BadRequest,
    "type.googleapis.com/google.rpc.BadRequest"
);
impl_any_traits!(
    RequestInfo,
    RequestInfo,
    "type.googleapis.com/google.rpc.RequestInfo"
);
impl_any_traits!(
    ResourceInfo,
    ResourceInfo,
    "type.googleapis.com/google.rpc.ResourceInfo"
);
impl_any_traits!(Help, Help, "type.googleapis.com/google.rpc.Help");
impl_any_traits!(
    LocalizedMessage,
    LocalizedMessage,
    "type.googleapis.com/google.rpc.LocalizedMessage"
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn error_info_roundtrip() {
        let original = ErrorInfo {
            reason: "API_DISABLED".into(),
            domain: "googleapis.com".into(),
            metadata: HashMap::from([("service".into(), "pubsub.googleapis.com".into())]),
        };
        let any = original.clone().into_any();
        assert_eq!(any.type_url, <ErrorInfo as IntoAny>::TYPE_URL);
        let decoded = ErrorInfo::from_any(&any).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn retry_info_roundtrip() {
        let original = RetryInfo {
            retry_delay: Some(prost_types::Duration {
                seconds: 5,
                nanos: 0,
            }),
        };
        let any = original.clone().into_any();
        let decoded = RetryInfo::from_any(&any).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn debug_info_roundtrip() {
        let original = DebugInfo {
            stack_entries: vec!["frame1".into(), "frame2".into()],
            detail: "segfault at 0x0".into(),
        };
        let any = original.clone().into_any();
        let decoded = DebugInfo::from_any(&any).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn bad_request_roundtrip() {
        let original = BadRequest {
            field_violations: vec![bad_request::FieldViolation {
                field: "email".into(),
                description: "not a valid email".into(),
                reason: "INVALID_FORMAT".into(),
                localized_message: None,
            }],
        };
        let any = original.clone().into_any();
        let decoded = BadRequest::from_any(&any).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn precondition_failure_roundtrip() {
        let original = PreconditionFailure {
            violations: vec![precondition_failure::Violation {
                r#type: "TOS".into(),
                subject: "google.com/cloud".into(),
                description: "Terms of service not accepted".into(),
            }],
        };
        let any = original.clone().into_any();
        let decoded = PreconditionFailure::from_any(&any).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn quota_failure_roundtrip() {
        let original = QuotaFailure {
            violations: vec![quota_failure::Violation {
                subject: "project:my-project".into(),
                description: "Daily limit exceeded".into(),
                api_service: "compute.googleapis.com".into(),
                quota_metric: "cpus".into(),
                quota_id: "CPUS-PER-PROJECT".into(),
                quota_dimensions: HashMap::from([("region".into(), "us-central1".into())]),
                quota_value: 100,
                future_quota_value: Some(200),
            }],
        };
        let any = original.clone().into_any();
        let decoded = QuotaFailure::from_any(&any).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn resource_info_roundtrip() {
        let original = ResourceInfo {
            resource_type: "sql table".into(),
            resource_name: "users".into(),
            owner: "user:admin@example.com".into(),
            description: "permission denied".into(),
        };
        let any = original.clone().into_any();
        let decoded = ResourceInfo::from_any(&any).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn request_info_roundtrip() {
        let original = RequestInfo {
            request_id: "req-12345".into(),
            serving_data: "frontend-42".into(),
        };
        let any = original.clone().into_any();
        let decoded = RequestInfo::from_any(&any).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn help_roundtrip() {
        let original = Help {
            links: vec![help::Link {
                description: "API docs".into(),
                url: "https://example.com/docs".into(),
            }],
        };
        let any = original.clone().into_any();
        let decoded = Help::from_any(&any).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn localized_message_roundtrip() {
        let original = LocalizedMessage {
            locale: "en-US".into(),
            message: "Something went wrong".into(),
        };
        let any = original.clone().into_any();
        let decoded = LocalizedMessage::from_any(&any).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn decode_any_dispatches_correctly() {
        let error_info = ErrorInfo {
            reason: "TEST".into(),
            domain: "test.com".into(),
            metadata: HashMap::new(),
        };
        let any = error_info.clone().into_any();
        let detail = decode_any(&any).unwrap();
        assert_eq!(detail, ErrorDetail::ErrorInfo(error_info));
    }

    #[test]
    fn decode_any_unknown_type_url() {
        let any = Any {
            type_url: "type.googleapis.com/some.Unknown".into(),
            value: vec![1, 2, 3],
        };
        let detail = decode_any(&any).unwrap();
        match detail {
            ErrorDetail::Unknown(a) => assert_eq!(a.type_url, "type.googleapis.com/some.Unknown"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }
}
