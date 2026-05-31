// xray-core & v2fly observatory protobuf messages
// Hand-written prost structs with exact field numbers matching upstream protobuf definitions

use prost::Message;
use std::time::Duration;

// ==========================================
// Prost Messages (from app/observatory/command/command.proto)
// ==========================================

#[derive(Clone, Message)]
pub struct GetOutboundStatusRequest {
    #[prost(string, tag = "1")]
    pub tag: String,
}

#[derive(Clone, Message)]
pub struct GetOutboundStatusResponse {
    #[prost(message, optional, tag = "1")]
    pub status: Option<ObservationResult>,
}

#[derive(Clone, Message)]
pub struct ObservationResult {
    #[prost(message, repeated, tag = "1")]
    pub status: Vec<OutboundStatus>,
}

#[derive(Clone, Message)]
pub struct OutboundStatus {
    #[prost(bool, tag = "1")]
    pub alive: bool,
    #[prost(int64, tag = "2")]
    pub delay: i64,
    #[prost(string, tag = "3")]
    pub last_error_reason: String,
    #[prost(string, tag = "4")]
    pub outbound_tag: String,
    #[prost(int64, tag = "5")]
    pub last_seen_time: i64,
    #[prost(int64, tag = "6")]
    pub last_try_time: i64,
    #[prost(message, optional, tag = "7")]
    pub health_ping: Option<HealthPingMeasurementResult>,
}

#[derive(Clone, Message)]
pub struct HealthPingMeasurementResult {
    #[prost(int64, tag = "1")]
    pub all: i64,
    #[prost(int64, tag = "2")]
    pub fail: i64,
    #[prost(int64, tag = "3")]
    pub deviation: i64,
    #[prost(int64, tag = "4")]
    pub average: i64,
    #[prost(int64, tag = "5")]
    pub max: i64,
    #[prost(int64, tag = "6")]
    pub min: i64,
}

// ==========================================
// Backend-neutral Adapter
// ==========================================

/// Backend-neutral result from an observatory query.
#[derive(Debug, Clone)]
pub struct ObservatoryStatus {
    pub outbound_tag: String,
    pub delay_ms: Option<u64>,
    pub alive: bool,
    pub last_error: Option<String>,
}

// ==========================================
// Error Type
// ==========================================

#[derive(Debug, thiserror::Error)]
pub enum ObservatoryError {
    #[error("gRPC transport error: {0}")]
    Transport(#[from] tonic::transport::Error),
    #[error("gRPC status error: {0}")]
    Status(#[from] tonic::Status),
    #[error("observatory query timed out after {0:?}")]
    Timeout(Duration),
}

// ==========================================
// Query Functions
// ==========================================

use http::uri::PathAndQuery;
use tonic::Request;
use tonic::client::Grpc;
use tonic::codec::ProstCodec;

const XRAY_SERVICE_PATH: &str =
    "/xray.core.app.observatory.command.ObservatoryService/GetOutboundStatus";
const V2RAY_SERVICE_PATH: &str =
    "/v2ray.core.app.observatory.command.ObservatoryService/GetOutboundStatus";

/// Query xray observatory service on localhost.
pub async fn query_xray_observatory(port: u16) -> Result<Vec<ObservatoryStatus>, ObservatoryError> {
    query_observatory(port, XRAY_SERVICE_PATH).await
}

/// Query v2ray observatory service on localhost.
pub async fn query_v2ray_observatory(
    port: u16,
) -> Result<Vec<ObservatoryStatus>, ObservatoryError> {
    query_observatory(port, V2RAY_SERVICE_PATH).await
}

async fn query_observatory(
    port: u16,
    service_path: &str,
) -> Result<Vec<ObservatoryStatus>, ObservatoryError> {
    let endpoint = tonic::transport::Endpoint::from_shared(format!("http://127.0.0.1:{port}"))
        .map_err(ObservatoryError::Transport)?
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5));

    let channel = endpoint
        .connect()
        .await
        .map_err(ObservatoryError::Transport)?;
    let mut grpc = Grpc::new(channel);

    let path: PathAndQuery = service_path.parse().map_err(|_| {
        tonic::Status::invalid_argument(format!("Invalid service path: {}", service_path))
    })?;
    let codec = ProstCodec::<GetOutboundStatusRequest, GetOutboundStatusResponse>::default();
    let request = Request::new(GetOutboundStatusRequest::default());

    let response = grpc
        .unary(request, path, codec)
        .await
        .map_err(ObservatoryError::Status)?;
    let inner = response.into_inner();

    Ok(inner
        .status
        .map(|result| {
            result
                .status
                .into_iter()
                .map(|s| ObservatoryStatus {
                    outbound_tag: s.outbound_tag,
                    delay_ms: if s.alive && s.delay > 0 {
                        Some(s.delay as u64)
                    } else {
                        None
                    },
                    alive: s.alive,
                    last_error: if s.last_error_reason.is_empty() {
                        None
                    } else {
                        Some(s.last_error_reason)
                    },
                })
                .collect()
        })
        .unwrap_or_default())
}

// ==========================================
// Unit Tests
// ==========================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protobuf_roundtrip_alive_with_delay() {
        let original = OutboundStatus {
            alive: true,
            delay: 150,
            last_error_reason: String::new(),
            outbound_tag: "proxy-1".to_string(),
            last_seen_time: 0,
            last_try_time: 0,
            health_ping: Some(HealthPingMeasurementResult {
                all: 10,
                fail: 0,
                deviation: 5,
                average: 150,
                max: 200,
                min: 100,
            }),
        };

        let mut buf = Vec::new();
        original.encode(&mut buf).unwrap();

        let decoded = OutboundStatus::decode(&mut buf.as_slice()).unwrap();

        assert_eq!(decoded.outbound_tag, original.outbound_tag);
        assert_eq!(decoded.alive, original.alive);
        assert_eq!(decoded.delay, original.delay);
        assert_eq!(decoded.last_error_reason, original.last_error_reason);
        assert!(decoded.health_ping.is_some());
        let ping = decoded.health_ping.unwrap();
        assert_eq!(ping.all, 10);
        assert_eq!(ping.average, 150);
    }

    #[test]
    fn test_protobuf_roundtrip_dead_with_error() {
        let original = OutboundStatus {
            alive: false,
            delay: 0,
            last_error_reason: "connection refused".to_string(),
            outbound_tag: "proxy-2".to_string(),
            last_seen_time: 1000,
            last_try_time: 2000,
            health_ping: None,
        };

        let mut buf = Vec::new();
        original.encode(&mut buf).unwrap();

        let decoded = OutboundStatus::decode(&mut buf.as_slice()).unwrap();

        assert_eq!(decoded.outbound_tag, original.outbound_tag);
        assert_eq!(decoded.alive, original.alive);
        assert_eq!(decoded.delay, original.delay);
        assert_eq!(decoded.last_error_reason, original.last_error_reason);
        assert_eq!(decoded.last_seen_time, original.last_seen_time);
        assert_eq!(decoded.last_try_time, original.last_try_time);
        assert!(decoded.health_ping.is_none());
    }

    #[test]
    fn test_protobuf_roundtrip_alive_zero_delay() {
        let original = OutboundStatus {
            alive: true,
            delay: 0,
            last_error_reason: String::new(),
            outbound_tag: "proxy-3".to_string(),
            last_seen_time: 3000,
            last_try_time: 3000,
            health_ping: None,
        };

        let mut buf = Vec::new();
        original.encode(&mut buf).unwrap();

        let decoded = OutboundStatus::decode(&mut buf.as_slice()).unwrap();

        assert_eq!(decoded.outbound_tag, original.outbound_tag);
        assert_eq!(decoded.alive, true);
        assert_eq!(decoded.delay, 0);
        assert_eq!(decoded.last_error_reason, "");
        assert_eq!(decoded.last_seen_time, 3000);
        assert_eq!(decoded.last_try_time, 3000);
        assert!(decoded.health_ping.is_none());
    }

    #[test]
    fn test_protobuf_roundtrip_not_alive_with_positive_delay() {
        let original = OutboundStatus {
            alive: false,
            delay: 5000,
            last_error_reason: "timeout".to_string(),
            outbound_tag: "proxy-4".to_string(),
            last_seen_time: 4000,
            last_try_time: 5000,
            health_ping: None,
        };

        let mut buf = Vec::new();
        original.encode(&mut buf).unwrap();

        let decoded = OutboundStatus::decode(&mut buf.as_slice()).unwrap();

        assert_eq!(decoded.alive, false);
        assert_eq!(decoded.delay, 5000);
        assert_eq!(decoded.last_seen_time, 4000);
        assert_eq!(decoded.last_try_time, 5000);
    }

    #[test]
    fn test_empty_response() {
        let response = GetOutboundStatusResponse {
            status: Some(ObservationResult { status: vec![] }),
        };

        let mut buf = Vec::new();
        response.encode(&mut buf).unwrap();

        let decoded = GetOutboundStatusResponse::decode(&mut buf.as_slice()).unwrap();

        assert!(decoded.status.is_some());
        assert!(decoded.status.unwrap().status.is_empty());
    }

    #[test]
    fn test_response_without_observation_result() {
        let response = GetOutboundStatusResponse { status: None };

        let mut buf = Vec::new();
        response.encode(&mut buf).unwrap();

        let decoded = GetOutboundStatusResponse::decode(&mut buf.as_slice()).unwrap();

        assert!(decoded.status.is_none());
    }

    #[test]
    fn test_multiple_statuses_roundtrip() {
        let response = GetOutboundStatusResponse {
            status: Some(ObservationResult {
                status: vec![
                    OutboundStatus {
                        alive: true,
                        delay: 200,
                        last_error_reason: String::new(),
                        outbound_tag: "proxy-alive".to_string(),
                        last_seen_time: 1000,
                        last_try_time: 1000,
                        health_ping: None,
                    },
                    OutboundStatus {
                        alive: false,
                        delay: 0,
                        last_error_reason: "network unreachable".to_string(),
                        outbound_tag: "proxy-dead".to_string(),
                        last_seen_time: 2000,
                        last_try_time: 3000,
                        health_ping: None,
                    },
                    OutboundStatus {
                        alive: true,
                        delay: 0,
                        last_error_reason: String::new(),
                        outbound_tag: "proxy-alive-zero".to_string(),
                        last_seen_time: 4000,
                        last_try_time: 4000,
                        health_ping: None,
                    },
                ],
            }),
        };

        let mut buf = Vec::new();
        response.encode(&mut buf).unwrap();

        let decoded = GetOutboundStatusResponse::decode(&mut buf.as_slice()).unwrap();

        assert!(decoded.status.is_some());
        let result = decoded.status.unwrap();
        assert_eq!(result.status.len(), 3);

        // First: alive with delay
        assert_eq!(result.status[0].outbound_tag, "proxy-alive");
        assert_eq!(result.status[0].alive, true);
        assert_eq!(result.status[0].delay, 200);

        // Second: dead with error
        assert_eq!(result.status[1].outbound_tag, "proxy-dead");
        assert_eq!(result.status[1].alive, false);
        assert_eq!(result.status[1].last_error_reason, "network unreachable");

        // Third: alive but zero delay
        assert_eq!(result.status[2].outbound_tag, "proxy-alive-zero");
        assert_eq!(result.status[2].alive, true);
        assert_eq!(result.status[2].delay, 0);
    }

    #[test]
    fn test_malformed_data_fails_gracefully() {
        let garbage = vec![0xFF, 0xFE, 0xFD, 0xFC, 0xFB, 0xFA, 0xF9, 0xF8];

        let result = OutboundStatus::decode(&mut garbage.as_slice());

        assert!(result.is_err());
    }

    #[test]
    fn test_request_message_encoding() {
        let request = GetOutboundStatusRequest {
            tag: "my-proxy".to_string(),
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let decoded = GetOutboundStatusRequest::decode(&mut buf.as_slice()).unwrap();

        assert_eq!(decoded.tag, "my-proxy");
    }

    #[test]
    fn test_default_request_is_empty_tag() {
        let request = GetOutboundStatusRequest::default();

        assert!(request.tag.is_empty());
    }
}
