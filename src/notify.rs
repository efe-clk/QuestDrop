//! B-phase delivery sinks. The publisher hands each outbox event to a Sink;
//! which sink runs is an env choice, so no code change is needed to go from
//! local logs to a real Discord channel.

use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct OutboxEvent {
    pub id: Uuid,
    pub kind: String,
    pub payload: serde_json::Value,
}

/// Delivery target for outbox events.
#[async_trait::async_trait]
pub trait Sink: Send + Sync {
    async fn send(&self, event: &OutboxEvent) -> Result<(), String>;
}

/// Local default: structured log line per event. Zero external deps.
pub struct LogSink;

#[async_trait::async_trait]
impl Sink for LogSink {
    async fn send(&self, event: &OutboxEvent) -> Result<(), String> {
        tracing::info!(
            event_id = %event.id,
            kind = %event.kind,
            payload = %event.payload,
            "outbox event"
        );
        Ok(())
    }
}

/// Discord channel via an incoming webhook URL (no bot token needed).
/// Set BOT_SINK=webhook + BOT_WEBHOOK_URL=https://discord.com/api/webhooks/...
pub struct WebhookSink {
    url: String,
    client: reqwest::Client,
}

impl WebhookSink {
    pub fn new(url: String) -> Self {
        let client = reqwest::Client::builder()
            // No timeout = one hung webhook stalls the batch forever.
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { url, client }
    }

    fn discord_body(event: &OutboxEvent) -> serde_json::Value {
        let text = match event.kind.as_str() {
            "swap.created" => format!(
                "New quest dropped: `{}`",
                event
                    .payload
                    .get("project_id")
                    .unwrap_or(&serde_json::Value::Null)
            ),
            "swap.completed" => format!(
                "Quest swapped: `{}`",
                event
                    .payload
                    .get("match_id")
                    .unwrap_or(&serde_json::Value::Null)
            ),
            other => format!("QuestDrop event `{other}`"),
        };
        serde_json::json!({ "content": text })
    }
}

#[async_trait::async_trait]
impl Sink for WebhookSink {
    async fn send(&self, event: &OutboxEvent) -> Result<(), String> {
        self.client
            .post(&self.url)
            .json(&Self::discord_body(event))
            .send()
            .await
            .map_err(|e| format!("webhook transport: {e}"))?
            .error_for_status()
            .map_err(|e| format!("webhook rejected: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_body_mentions_ids() {
        let e = OutboxEvent {
            id: Uuid::nil(),
            kind: "swap.completed".into(),
            payload: serde_json::json!({"match_id": "abc"}),
        };
        let b = WebhookSink::discord_body(&e);
        assert!(b["content"].as_str().unwrap().contains("abc"));
    }
}
