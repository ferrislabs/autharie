use serde::Serialize;

pub const DECISION_TARGET: &str = "agent_decision";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DecisionRecord {
    pub kind: &'static str,
    pub time: String,
    pub server: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub subject_type: &'static str,
    pub subject_id: String,
    pub agent: String,
    pub delegated: bool,
    pub decision: &'static str,
    pub policies: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    pub request_id: String,
}

pub const KIND: &str = "agent_decision";

pub trait DecisionSink: Send + Sync {
    fn record(&self, record: &DecisionRecord);
}

pub struct TracingSink;

impl DecisionSink for TracingSink {
    fn record(&self, record: &DecisionRecord) {
        match serde_json::to_string(record) {
            Ok(line) => tracing::info!(target: "agent_decision", "{line}"),
            Err(error) => tracing::error!(%error, "decision record not serialisable"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> DecisionRecord {
        DecisionRecord {
            kind: KIND,
            time: "2026-10-22T10:00:00Z".into(),
            server: "autharie".into(),
            method: "tools/call".into(),
            tool: Some("list_deployments".into()),
            subject_type: "User",
            subject_id: "alice".into(),
            agent: "claude".into(),
            delegated: true,
            decision: "allow",
            policies: vec!["p1".into()],
            reason: None,
            request_id: "r1".into(),
        }
    }

    #[test]
    fn the_record_has_the_frozen_shape() {
        let value = serde_json::to_value(record()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "kind": "agent_decision",
                "time": "2026-10-22T10:00:00Z",
                "server": "autharie",
                "method": "tools/call",
                "tool": "list_deployments",
                "subject_type": "User",
                "subject_id": "alice",
                "agent": "claude",
                "delegated": true,
                "decision": "allow",
                "policies": ["p1"],
                "request_id": "r1",
            })
        );
    }

    #[test]
    fn a_denial_keeps_its_reason_and_a_list_has_no_tool() {
        let mut denied = record();
        denied.tool = None;
        denied.decision = "deny";
        denied.reason = Some("pdp_timeout");
        let value = serde_json::to_value(denied).unwrap();
        assert!(value.get("tool").is_none());
        assert_eq!(value["reason"], "pdp_timeout");
        assert_eq!(value["decision"], "deny");
    }
}
