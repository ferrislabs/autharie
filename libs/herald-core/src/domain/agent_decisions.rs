use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::time::{interval, sleep};
use tracing::{debug, warn};

use super::entities::deployment::DeploymentId;
use super::entities::logs::{LogLine, OrganisationId};
use super::entities::shard::ShardConfig;
use super::log_index::{LogIndexDocument, fingerprint};
use super::ports::{GatewayLogSource, LogIndexSink, TenantSource};

pub const AGENT_DECISION_KIND: &str = "agent_decision";
pub const GATEWAY_SOURCE: &str = "mcp-gateway";

const DIRECTORY_REFRESH: Duration = Duration::from_secs(30);
const RELIST_INTERVAL: Duration = Duration::from_secs(5 * 60);
const RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    pub organisation_id: OrganisationId,
    pub deployment_id: DeploymentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DecisionRecord {
    pub kind: String,
    #[serde(default)]
    pub time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub server: String,
    pub resource: String,
    pub method: String,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub subject_type: String,
    #[serde(default)]
    pub subject_id: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub delegated: bool,
    pub decision: String,
    #[serde(default)]
    pub policies: Vec<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub request_id: String,
}

pub fn parse_decision_line(line: &str) -> Option<DecisionRecord> {
    let line = line.trim();
    if !line.starts_with('{') {
        return None;
    }
    let record: DecisionRecord = serde_json::from_str(line).ok()?;
    (record.kind == AGENT_DECISION_KIND).then_some(record)
}

pub fn host_of(resource: &str) -> Option<String> {
    let rest = resource
        .strip_prefix("https://")
        .or_else(|| resource.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

#[derive(Debug, Default)]
pub struct TenantDirectory {
    by_host: HashMap<String, Tenant>,
}

impl TenantDirectory {
    pub fn new(entries: impl IntoIterator<Item = (String, Tenant)>) -> Self {
        Self {
            by_host: entries
                .into_iter()
                .map(|(host, tenant)| (host.to_ascii_lowercase(), tenant))
                .collect(),
        }
    }

    pub fn resolve(&self, resource: &str) -> Option<&Tenant> {
        self.by_host.get(&host_of(resource)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentDecisionFields {
    pub kind: &'static str,
    pub decision: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub subject_type: String,
    pub subject_id: String,
    pub agent: String,
    pub server: String,
    pub resource: String,
    pub delegated: bool,
    pub policies: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub request_id: String,
}

impl LogIndexDocument {
    pub fn from_decision(tenant: &Tenant, record: DecisionRecord, seen_at: DateTime<Utc>) -> Self {
        let message = match &record.tool {
            Some(tool) => format!(
                "{} {} {} by {}",
                record.decision, record.method, tool, record.subject_id
            ),
            None => format!(
                "{} {} by {}",
                record.decision, record.method, record.subject_id
            ),
        };

        Self {
            timestamp: record.time.unwrap_or(seen_at),
            organisation_id: tenant.organisation_id.clone(),
            deployment_id: tenant.deployment_id.clone(),
            source: GATEWAY_SOURCE.to_string(),
            level: if record.decision == "allow" {
                "info"
            } else {
                "warn"
            }
            .to_string(),
            fingerprint: fingerprint(&message),
            message,
            delivery: None,
            agent_decision: Some(AgentDecisionFields {
                kind: AGENT_DECISION_KIND,
                decision: record.decision,
                method: record.method,
                tool: record.tool,
                subject_type: record.subject_type,
                subject_id: record.subject_id,
                agent: record.agent,
                server: record.server,
                resource: record.resource,
                delegated: record.delegated,
                policies: record.policies,
                reason: record.reason,
                request_id: record.request_id,
            }),
        }
    }
}

#[derive(Debug, Default)]
pub struct DecisionPipeline {
    directory: TenantDirectory,
    dropped_unknown_host: AtomicU64,
}

impl DecisionPipeline {
    pub fn replace_directory(&mut self, directory: TenantDirectory) {
        self.directory = directory;
    }

    pub fn dropped_unknown_host(&self) -> u64 {
        self.dropped_unknown_host.load(Ordering::Relaxed)
    }

    pub fn document_for(
        &self,
        line: &LogLine,
        shard: &ShardConfig,
    ) -> Option<(OrganisationId, LogIndexDocument)> {
        let record = parse_decision_line(&line.message)?;

        let Some(tenant) = self.directory.resolve(&record.resource) else {
            let total = self.dropped_unknown_host.fetch_add(1, Ordering::Relaxed) + 1;
            debug!(
                resource = %record.resource,
                dropped_unknown_host = total,
                "an agent decision names a host no instance on this cluster serves; dropping it"
            );
            return None;
        };

        if !shard.owns_deployment(&tenant.deployment_id) {
            return None;
        }

        let organisation_id = tenant.organisation_id.clone();
        let document = LogIndexDocument::from_decision(tenant, record, line.at);
        Some((organisation_id, document))
    }
}

pub async fn run_decision_shipping<L, T>(
    logs: Arc<L>,
    tenants: Arc<T>,
    sink: Arc<dyn LogIndexSink>,
    shard: ShardConfig,
) where
    L: GatewayLogSource,
    T: TenantSource,
{
    let mut pipeline = DecisionPipeline::default();
    refresh(&mut pipeline, tenants.as_ref()).await;

    loop {
        let mut lines = match logs.follow().await {
            Ok(lines) => lines,
            Err(err) => {
                warn!(%err, "could not read the MCP gateway's pods; retrying");
                sleep(RETRY_DELAY).await;
                refresh(&mut pipeline, tenants.as_ref()).await;
                continue;
            }
        };

        let mut refresh_tick = interval(DIRECTORY_REFRESH);
        let relist = sleep(RELIST_INTERVAL);
        tokio::pin!(relist);

        loop {
            tokio::select! {
                received = lines.recv() => match received {
                    Some(line) => {
                        if let Some((organisation_id, document)) = pipeline.document_for(&line, &shard)
                            && let Err(err) = sink.ship(organisation_id, vec![document]).await
                        {
                            warn!(%err, "failed to ship an agent decision to the search index");
                        }
                    }
                    None => break,
                },
                _ = refresh_tick.tick() => refresh(&mut pipeline, tenants.as_ref()).await,
                _ = &mut relist => break,
            }
        }
    }
}

async fn refresh<T: TenantSource>(pipeline: &mut DecisionPipeline, tenants: &T) {
    match tenants.tenants().await {
        Ok(entries) => pipeline.replace_directory(TenantDirectory::new(entries)),
        Err(err) => warn!(%err, "could not refresh the instances agent decisions resolve against"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::{MockGatewayLogSource, MockLogIndexSink, MockTenantSource};
    use serde_json::json;
    use tokio::sync::mpsc;

    fn tenant() -> Tenant {
        Tenant {
            organisation_id: OrganisationId::new("org-1"),
            deployment_id: DeploymentId::new("dep-1"),
        }
    }

    fn pipeline() -> DecisionPipeline {
        let mut pipeline = DecisionPipeline::default();
        pipeline.replace_directory(TenantDirectory::new([(
            "Auth.Acme.com".to_string(),
            tenant(),
        )]));
        pipeline
    }

    fn decision_json(resource: &str) -> String {
        json!({
            "kind": "agent_decision",
            "time": "2026-10-22T10:00:00Z",
            "server": "ferriskey",
            "resource": resource,
            "method": "tools/call",
            "tool": "list_users",
            "subject_type": "User",
            "subject_id": "alice",
            "agent": "claude",
            "delegated": true,
            "decision": "deny",
            "policies": ["p1"],
            "reason": "no_policy",
            "request_id": "r1"
        })
        .to_string()
    }

    fn line(message: &str) -> LogLine {
        LogLine {
            at: "2026-10-22T10:00:05Z".parse().unwrap(),
            source: "mcp-gateway".to_string(),
            message: message.to_string(),
        }
    }

    fn shard() -> ShardConfig {
        ShardConfig::new(0, 1)
    }

    #[test]
    fn a_decision_line_is_kept() {
        let record = parse_decision_line(&decision_json("https://auth.acme.com/mcp")).unwrap();
        assert_eq!(record.decision, "deny");
        assert_eq!(record.tool.as_deref(), Some("list_users"));
        assert_eq!(record.policies, vec!["p1"]);
    }

    #[test]
    fn a_plain_log_line_is_dropped() {
        assert!(parse_decision_line("2026-10-22T10:00:00Z  INFO mcp_gateway: listening").is_none());
    }

    #[test]
    fn json_of_another_kind_is_dropped() {
        assert!(parse_decision_line(r#"{"kind":"something_else","resource":"https://a/mcp","method":"m","decision":"allow"}"#).is_none());
        assert!(parse_decision_line(r#"{"msg":"hello"}"#).is_none());
    }

    #[test]
    fn malformed_json_is_dropped() {
        assert!(parse_decision_line(r#"{"kind":"agent_decision","resource":"#).is_none());
        assert!(parse_decision_line(r#"{"kind":"agent_decision"}"#).is_none());
    }

    #[test]
    fn the_host_is_read_from_the_resource() {
        assert_eq!(
            host_of("https://Auth.Acme.com/mcp").as_deref(),
            Some("auth.acme.com")
        );
        assert_eq!(
            host_of("https://auth.acme.com:8443/mcp").as_deref(),
            Some("auth.acme.com")
        );
        assert_eq!(host_of("auth.acme.com/mcp"), None);
        assert_eq!(host_of("https:///mcp"), None);
    }

    #[test]
    fn a_resource_resolves_to_the_instance_serving_its_host() {
        let pipeline = pipeline();
        assert_eq!(
            pipeline.directory.resolve("https://auth.acme.com/mcp"),
            Some(&tenant())
        );
        assert_eq!(pipeline.directory.resolve("https://other.com/mcp"), None);
    }

    #[test]
    fn the_document_carries_the_scoping_and_the_decision() {
        let pipeline = pipeline();
        let (organisation_id, document) = pipeline
            .document_for(&line(&decision_json("https://auth.acme.com/mcp")), &shard())
            .unwrap();

        assert_eq!(organisation_id, OrganisationId::new("org-1"));
        let value = serde_json::to_value(&document).unwrap();
        assert_eq!(value["kind"], "agent_decision");
        assert_eq!(value["organisation_id"], "org-1");
        assert_eq!(value["deployment_id"], "dep-1");
        assert_eq!(value["source"], "mcp-gateway");
        assert_eq!(value["level"], "warn");
        assert_eq!(value["timestamp"], "2026-10-22T10:00:00Z");
        assert_eq!(value["decision"], "deny");
        assert_eq!(value["tool"], "list_users");
        assert_eq!(value["subject_id"], "alice");
        assert_eq!(value["agent"], "claude");
        assert_eq!(value["server"], "ferriskey");
        assert_eq!(value["resource"], "https://auth.acme.com/mcp");
        assert_eq!(value["method"], "tools/call");
        assert_eq!(value["policies"], json!(["p1"]));
        assert_eq!(value["reason"], "no_policy");
        assert_eq!(value["delegated"], true);
        assert_eq!(value["request_id"], "r1");
        assert!(value.get("delivery").is_none());
    }

    #[test]
    fn an_unknown_host_is_dropped_and_counted() {
        let pipeline = pipeline();
        assert!(
            pipeline
                .document_for(
                    &line(&decision_json("https://nobody.example/mcp")),
                    &shard()
                )
                .is_none()
        );
        assert_eq!(pipeline.dropped_unknown_host(), 1);
    }

    #[test]
    fn a_plain_line_is_never_counted_as_unknown_host() {
        let pipeline = pipeline();
        assert!(
            pipeline
                .document_for(&line("INFO started"), &shard())
                .is_none()
        );
        assert_eq!(pipeline.dropped_unknown_host(), 0);
    }

    #[test]
    fn a_deployment_another_shard_owns_is_skipped() {
        let pipeline = pipeline();
        let owner = shard_owning(&tenant().deployment_id);
        let other = ShardConfig::new((owner.shard_index + 1) % 2, 2);
        assert!(
            pipeline
                .document_for(&line(&decision_json("https://auth.acme.com/mcp")), &other)
                .is_none()
        );
        assert!(
            pipeline
                .document_for(&line(&decision_json("https://auth.acme.com/mcp")), &owner)
                .is_some()
        );
        assert_eq!(pipeline.dropped_unknown_host(), 0);
    }

    fn shard_owning(id: &DeploymentId) -> ShardConfig {
        [ShardConfig::new(0, 2), ShardConfig::new(1, 2)]
            .into_iter()
            .find(|shard| shard.owns_deployment(id))
            .unwrap()
    }

    #[tokio::test]
    async fn only_decisions_of_known_instances_reach_the_sink() {
        let (line_tx, line_rx) = mpsc::channel(8);
        let line_rx = std::sync::Mutex::new(Some(line_rx));
        let mut logs = MockGatewayLogSource::new();
        logs.expect_follow().returning(move || {
            let received = line_rx.lock().unwrap().take();
            Box::pin(async move {
                match received {
                    Some(receiver) => Ok(receiver),
                    None => Ok(mpsc::channel(1).1),
                }
            })
        });

        let mut tenants = MockTenantSource::new();
        tenants
            .expect_tenants()
            .returning(|| Box::pin(async { Ok(vec![("auth.acme.com".to_string(), tenant())]) }));

        let (shipped_tx, mut shipped_rx) = mpsc::unbounded_channel();
        let mut sink = MockLogIndexSink::new();
        sink.expect_ship().returning(move |org, documents| {
            let shipped_tx = shipped_tx.clone();
            Box::pin(async move {
                let _ = shipped_tx.send((org, documents));
                Ok(())
            })
        });

        let task = tokio::spawn(run_decision_shipping(
            Arc::new(logs),
            Arc::new(tenants),
            Arc::new(sink) as Arc<dyn LogIndexSink>,
            shard(),
        ));

        line_tx.send(line("INFO plain")).await.unwrap();
        line_tx.send(line("{not json")).await.unwrap();
        line_tx
            .send(line(&decision_json("https://nobody.example/mcp")))
            .await
            .unwrap();
        line_tx
            .send(line(&decision_json("https://auth.acme.com/mcp")))
            .await
            .unwrap();

        let (org, documents) = shipped_rx.recv().await.unwrap();
        assert_eq!(org, OrganisationId::new("org-1"));
        assert_eq!(documents.len(), 1);
        assert!(shipped_rx.try_recv().is_err());
        task.abort();
    }
}
