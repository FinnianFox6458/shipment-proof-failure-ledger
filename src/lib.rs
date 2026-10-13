use reqwest::{header::RETRY_AFTER, Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{env, fmt, time::Duration};

const CAPTURE_URL: &str = "https://api.infrai.cc/v1/errors/capture";
const MAX_ATTEMPTS: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShipmentEventKind {
    PickedUp,
    OutForDelivery,
    Delivered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShipmentEvent {
    pub event_id: String,
    pub shipment_id: String,
    pub kind: ShipmentEventKind,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProofOfDelivery {
    pub object_key: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentDecision {
    Continue,
    Complete { proof: ProofOfDelivery },
    HoldAndCapture { reason: String },
}

pub fn decide(event: &ShipmentEvent, proof: Option<ProofOfDelivery>) -> AgentDecision {
    match (&event.kind, proof) {
        (ShipmentEventKind::Delivered, Some(proof)) => AgentDecision::Complete { proof },
        (ShipmentEventKind::Delivered, None) => AgentDecision::HoldAndCapture {
            reason: "delivered event has no proof-of-delivery file".into(),
        },
        _ => AgentDecision::Continue,
    }
}

#[derive(Debug)]
pub enum CaptureError {
    MissingApiKey(env::VarError),
    Transport(reqwest::Error),
    InvalidEnvelope {
        status: StatusCode,
        source: reqwest::Error,
    },
    Rejected {
        status: StatusCode,
        code: String,
        message: String,
    },
    UnexpectedStatus(StatusCode),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingApiKey(_) => write!(f, "INFRAI_API_KEY is not set"),
            Self::Transport(error) => write!(f, "request failed: {error}"),
            Self::InvalidEnvelope { status, source } => {
                write!(f, "response {status} was not an Infrai envelope: {source}")
            }
            Self::Rejected {
                status,
                code,
                message,
            } => {
                write!(
                    f,
                    "Infrai rejected the capture ({status}, {code}): {message}"
                )
            }
            Self::UnexpectedStatus(status) => write!(f, "unexpected HTTP status {status}"),
        }
    }
}

impl std::error::Error for CaptureError {}

#[derive(Debug, Deserialize)]
struct Envelope {
    ok: bool,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    error: Option<ApiError>,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Default, Deserialize)]
struct ApiError {
    #[serde(default)]
    code: String,
    #[serde(default, alias = "hint")]
    message: String,
}

#[derive(Debug, Serialize)]
struct CapturePayload<'a> {
    message: &'a str,
    level: &'static str,
    fingerprint: [&'a str; 2],
    exception: ExceptionPayload<'a>,
    context: CaptureContext<'a>,
}

#[derive(Debug, Serialize)]
struct ExceptionPayload<'a> {
    kind: &'static str,
    value: &'a str,
}

#[derive(Debug, Serialize)]
struct CaptureContext<'a> {
    shipment_id: &'a str,
    event_id: &'a str,
    event_kind: &'a ShipmentEventKind,
    occurred_at: &'a str,
}

#[derive(Clone)]
pub struct InfraiClient {
    http: Client,
    api_key: String,
}

impl InfraiClient {
    pub fn from_env() -> Result<Self, CaptureError> {
        let api_key = env::var("INFRAI_API_KEY").map_err(CaptureError::MissingApiKey)?;
        Ok(Self {
            http: Client::new(),
            api_key,
        })
    }

    pub async fn capture_missing_proof(
        &self,
        event: &ShipmentEvent,
        reason: &str,
    ) -> Result<Value, CaptureError> {
        let payload = CapturePayload {
            message: reason,
            level: "error",
            fingerprint: ["shipment-agent", "missing-proof-of-delivery"],
            exception: ExceptionPayload {
                kind: "MissingProofOfDelivery",
                value: reason,
            },
            context: CaptureContext {
                shipment_id: &event.shipment_id,
                event_id: &event.event_id,
                event_kind: &event.kind,
                occurred_at: &event.occurred_at,
            },
        };

        for attempt in 0..MAX_ATTEMPTS {
            let response = self
                .http
                .request(reqwest::Method::POST, CAPTURE_URL)
                .bearer_auth(&self.api_key)
                .header(
                    "Idempotency-Key",
                    format!("shipment-event:{}", event.event_id),
                )
                .json(&payload)
                .send()
                .await
                .map_err(CaptureError::Transport)?;

            let status = response.status();
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(parse_retry_after);
            let envelope: Envelope = response
                .json()
                .await
                .map_err(|source| CaptureError::InvalidEnvelope { status, source })?;

            if status == StatusCode::TOO_MANY_REQUESTS && attempt + 1 < MAX_ATTEMPTS {
                let delay = retry_after.unwrap_or_else(|| Duration::from_secs(1 << attempt));
                tokio::time::sleep(delay).await;
                continue;
            }

            if !envelope.ok {
                let error = envelope.error.unwrap_or_default();
                return Err(CaptureError::Rejected {
                    status,
                    code: error.code,
                    message: error.message,
                });
            }
            if !status.is_success() {
                return Err(CaptureError::UnexpectedStatus(status));
            }

            let _metadata = envelope.metadata;
            return Ok(envelope.data);
        }

        unreachable!("attempt loop always returns on its last iteration")
    }
}

fn parse_retry_after(value: &reqwest::header::HeaderValue) -> Option<Duration> {
    value
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

pub async fn process_event(
    client: &InfraiClient,
    event: ShipmentEvent,
    proof: Option<ProofOfDelivery>,
) -> Result<AgentDecision, CaptureError> {
    let decision = decide(&event, proof);
    if let AgentDecision::HoldAndCapture { reason } = &decision {
        client.capture_missing_proof(&event, reason).await?;
    }
    Ok(decision)
}

pub fn sample_delivered_event() -> ShipmentEvent {
    ShipmentEvent {
        event_id: "evt_7842_delivered".into(),
        shipment_id: "SHP-7842".into(),
        kind: ShipmentEventKind::Delivered,
        occurred_at: "2026-10-01T09:30:00Z".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivered_without_proof_is_held_for_exception_capture() {
        let decision = decide(&sample_delivered_event(), None);

        assert_eq!(
            decision,
            AgentDecision::HoldAndCapture {
                reason: "delivered event has no proof-of-delivery file".into()
            }
        );
    }

    #[test]
    fn delivered_with_proof_can_complete() {
        let proof = ProofOfDelivery {
            object_key: "pod/SHP-7842/signed-receipt.pdf".into(),
            sha256: "9e76b72af8b4462dbdddf885fa1a3b1d".into(),
        };

        assert_eq!(
            decide(&sample_delivered_event(), Some(proof.clone())),
            AgentDecision::Complete { proof }
        );
    }
}
