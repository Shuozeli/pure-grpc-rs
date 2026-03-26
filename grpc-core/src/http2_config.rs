//! Shared HTTP/2 connection-level settings for both server and client.

use std::time::Duration;

/// HTTP/2 connection-level settings shared between server and client.
///
/// Fields that are `None` use the hyper/h2 defaults.
#[derive(Debug, Clone, Default)]
pub struct Http2Config {
    /// Initial window size for each HTTP/2 stream.
    pub initial_stream_window_size: Option<u32>,
    /// Initial window size for the HTTP/2 connection.
    pub initial_connection_window_size: Option<u32>,
    /// Enable adaptive flow control.
    pub adaptive_window: Option<bool>,
    /// Maximum HTTP/2 frame size.
    pub max_frame_size: Option<u32>,
    /// Interval between HTTP/2 PING frames.
    pub keep_alive_interval: Option<Duration>,
    /// Timeout for HTTP/2 PING acknowledgement.
    pub keep_alive_timeout: Option<Duration>,
}
