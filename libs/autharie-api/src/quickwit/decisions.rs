use autharie_core::{
    CoreError,
    agents::{AgentDecision, DecisionQuery, DecisionVerdict, decision::AgentDecisionIndex},
    organisation::OrganisationId,
};
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::Deserialize;

use super::logs::QuickwitLogSearchIndex;

#[derive(Debug, Deserialize)]
struct DecisionHit {
    timestamp: DateTime<Utc>,
    #[serde(default)]
    server: String,
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    method: String,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    subject_type: String,
    #[serde(default)]
    subject_id: String,
    #[serde(default)]
    agent: String,
    #[serde(default)]
    delegated: bool,
    decision: String,
    #[serde(default)]
    policies: Vec<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    request_id: String,
}

#[derive(Debug, Deserialize)]
struct DecisionResponse {
    #[serde(default)]
    hits: Vec<serde_json::Value>,
}

fn as_phrase(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "'"))
}

fn build_query(query: &DecisionQuery) -> String {
    let mut clauses = vec![
        "kind:agent_decision".to_string(),
        format!(
            "deployment_id:{}",
            as_phrase(&query.deployment_id.0.to_string())
        ),
    ];

    if let Some(decision) = query.decision {
        clauses.push(format!("decision:{}", as_phrase(decision.as_str())));
    }
    if let Some(tool) = &query.tool {
        clauses.push(format!("tool:{}", as_phrase(tool)));
    }
    if let Some(subject) = &query.subject {
        clauses.push(format!("subject_id:{}", as_phrase(subject)));
    }

    clauses.join(" AND ")
}

fn decisions(hits: Vec<DecisionHit>, query: &DecisionQuery) -> Vec<AgentDecision> {
    let mut found: Vec<AgentDecision> = hits
        .into_iter()
        .filter(|hit| {
            query
                .tool
                .as_ref()
                .is_none_or(|tool| hit.tool.as_ref() == Some(tool))
        })
        .filter(|hit| {
            query
                .subject
                .as_ref()
                .is_none_or(|subject| &hit.subject_id == subject)
        })
        .filter_map(|hit| {
            let decision = DecisionVerdict::parse(&hit.decision)?;
            if query.decision.is_some_and(|wanted| wanted != decision) {
                return None;
            }

            Some(AgentDecision {
                kind: "agent_decision".to_string(),
                time: hit.timestamp,
                deployment_id: query.deployment_id.0,
                server: hit.server,
                resource: hit.resource,
                method: hit.method,
                tool: hit.tool,
                subject_type: hit.subject_type,
                subject_id: hit.subject_id,
                agent: hit.agent,
                delegated: hit.delegated,
                decision,
                policies: hit.policies,
                reason: hit.reason,
                request_id: hit.request_id,
            })
        })
        .collect();

    found.sort_by(|a, b| {
        b.time
            .cmp(&a.time)
            .then_with(|| a.request_id.cmp(&b.request_id))
    });
    found.truncate(query.limit);
    found
}

impl AgentDecisionIndex for QuickwitLogSearchIndex {
    async fn decisions(
        &self,
        organisation_id: OrganisationId,
        query: DecisionQuery,
    ) -> Result<Vec<AgentDecision>, CoreError> {
        let mut body = serde_json::json!({
            "query": build_query(&query),
            "max_hits": query.limit,
            "sort_by_field": "-timestamp",
        });
        if let Some(from) = query.from {
            body["start_timestamp"] = from.timestamp().into();
        }
        if let Some(to) = query.to {
            body["end_timestamp"] = to.timestamp().into();
        }

        let response = self
            .client
            .post(self.search_url(organisation_id))
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                CoreError::InternalError(format!("failed to search the decision index: {err}"))
            })?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(CoreError::InternalError(format!(
                "quickwit search failed with status {status}: {body}"
            )));
        }

        let parsed: DecisionResponse = response.json().await.map_err(|err| {
            CoreError::InternalError(format!("failed to parse quickwit's response: {err}"))
        })?;

        let hits = parsed
            .hits
            .into_iter()
            .filter_map(|hit| serde_json::from_value::<DecisionHit>(hit).ok())
            .collect();

        Ok(decisions(hits, &query))
    }
}

impl AgentDecisionIndex for &QuickwitLogSearchIndex {
    async fn decisions(
        &self,
        organisation_id: OrganisationId,
        query: DecisionQuery,
    ) -> Result<Vec<AgentDecision>, CoreError> {
        (*self).decisions(organisation_id, query).await
    }
}

#[cfg(test)]
mod tests {
    use autharie_core::deployments::DeploymentId;
    use chrono::Duration;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn query() -> DecisionQuery {
        DecisionQuery {
            deployment_id: DeploymentId(Uuid::from_u128(1)),
            decision: None,
            tool: None,
            subject: None,
            from: None,
            to: None,
            limit: 50,
        }
    }

    fn hit(request_id: &str, decision: &str, tool: &str, subject: &str, at: i64) -> DecisionHit {
        serde_json::from_value(json!({
            "timestamp": (DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(at)).to_rfc3339(),
            "server": "ferriskey",
            "method": "tools/call",
            "tool": tool,
            "subject_type": "User",
            "subject_id": subject,
            "agent": "claude",
            "delegated": true,
            "decision": decision,
            "policies": ["p1"],
            "request_id": request_id,
        }))
        .unwrap()
    }

    #[test]
    fn the_query_is_scoped_to_agent_decisions_of_one_deployment() {
        let text = build_query(&query());

        assert!(text.starts_with("kind:agent_decision AND "));
        assert!(text.contains(&format!("deployment_id:\"{}\"", Uuid::from_u128(1))));
        assert!(!text.contains("decision:"));
    }

    #[test]
    fn each_filter_becomes_one_clause_on_its_own_field() {
        let mut wanted = query();
        wanted.decision = Some(DecisionVerdict::Deny);
        wanted.tool = Some("list_users".to_string());
        wanted.subject = Some("alice".to_string());

        let text = build_query(&wanted);

        assert!(text.contains("AND decision:\"deny\""));
        assert!(text.contains("AND tool:\"list_users\""));
        assert!(text.contains("AND subject_id:\"alice\""));
    }

    #[test]
    fn a_quote_in_a_filter_cannot_close_the_phrase() {
        let mut wanted = query();
        wanted.tool = Some("x\" OR kind:log".to_string());

        assert!(build_query(&wanted).contains("tool:\"x' OR kind:log\""));
    }

    #[test]
    fn hits_come_back_newest_first_within_the_limit_with_the_deployment() {
        let mut limited = query();
        limited.limit = 2;

        let found = decisions(
            vec![
                hit("a", "allow", "t", "u", 10),
                hit("b", "deny", "t", "u", 30),
                hit("c", "allow", "t", "u", 20),
            ],
            &limited,
        );

        let ids: Vec<_> = found.iter().map(|d| d.request_id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c"]);
        assert_eq!(found[0].deployment_id, Uuid::from_u128(1));
        assert_eq!(found[0].decision, DecisionVerdict::Deny);
    }

    #[test]
    fn a_tokenised_near_match_is_dropped_by_the_exact_filters() {
        let mut wanted = query();
        wanted.tool = Some("list_users".to_string());
        wanted.subject = Some("alice".to_string());
        wanted.decision = Some(DecisionVerdict::Allow);

        let found = decisions(
            vec![
                hit("keep", "allow", "list_users", "alice", 1),
                hit("tool", "allow", "list_users_all", "alice", 2),
                hit("subject", "allow", "list_users", "alice2", 3),
                hit("verdict", "deny", "list_users", "alice", 4),
                hit("unknown", "perhaps", "list_users", "alice", 5),
            ],
            &wanted,
        );

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].request_id, "keep");
    }
}
