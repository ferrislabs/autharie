use std::fmt::Write;

use autharie_crds::v1alpha::identity_instance::IdentityInstance;

use super::{FERRISKEY_SERVER_NAME, GATEWAY_CLIENT_ID, evaluator_audience, servers};
use crate::infrastructure::ferriskey_admin::api_base_url;
use crate::infrastructure::identity_instance::{ferriskey_api_base_url, public_origin};

const FERRISKEY_MCP_SERVICE: &str = "ferriskey-mcp";
const FERRISKEY_MCP_PORT: u16 = 8080;
const JWKS_REFRESH_SECS: u64 = 300;
const PDP_TIMEOUT_MS: u64 = 2000;

#[derive(Clone, PartialEq, Eq)]
pub struct Upstream {
    pub canonical_uri: String,
    pub issuer: String,
    pub jwks_uri: String,
    pub evaluation_url: String,
    pub evaluations_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub token_url: String,
    pub audience: String,
    pub upstream_url: String,
    pub server_name: String,
    pub scopes_supported: Vec<String>,
    pub egress: Egress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Egress {
    Cluster,
    Public,
}

impl std::fmt::Debug for Upstream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Upstream")
            .field("canonical_uri", &self.canonical_uri)
            .field("server_name", &self.server_name)
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsEnvironment {
    pub release_namespace: String,
    pub public_https_port: Option<u16>,
}

impl AgentsEnvironment {
    pub fn new(release_namespace: impl Into<String>, public_https_port: Option<u16>) -> Self {
        Self {
            release_namespace: release_namespace.into(),
            public_https_port,
        }
    }

    pub fn canonical_uri(&self, instance: &IdentityInstance) -> String {
        format!(
            "{}/mcp",
            public_origin(&instance.spec.hostname, self.public_https_port)
        )
    }

    fn public_api_base(&self, instance: &IdentityInstance) -> String {
        let base = ferriskey_api_base_url(instance, self.public_https_port);
        let base = base.trim_end_matches('/');
        if base.starts_with('/') {
            format!(
                "{}{base}",
                public_origin(&instance.spec.hostname, self.public_https_port)
            )
        } else {
            base.to_string()
        }
    }

    pub fn upstream(&self, instance: &IdentityInstance, secret: &str) -> Option<Upstream> {
        let agents = instance.spec.agents.as_ref()?;
        let name = instance.metadata.name.as_deref()?;
        let namespace = instance.metadata.namespace.as_deref()?;
        let realm = agents.realm.as_str();
        let cluster = format!("{}/realms/{realm}", api_base_url(name, namespace));

        Some(Upstream {
            canonical_uri: self.canonical_uri(instance),
            issuer: format!("{}/realms/{realm}", self.public_api_base(instance)),
            jwks_uri: format!("{cluster}/protocol/openid-connect/certs"),
            evaluation_url: format!("{cluster}/authzen/access/v1/evaluation"),
            evaluations_url: format!("{cluster}/authzen/access/v1/evaluations"),
            client_id: GATEWAY_CLIENT_ID.to_string(),
            client_secret: secret.to_string(),
            token_url: format!("{cluster}/protocol/openid-connect/token"),
            audience: evaluator_audience(realm),
            upstream_url: format!(
                "http://{FERRISKEY_MCP_SERVICE}.{}.svc:{FERRISKEY_MCP_PORT}/instances/{namespace}/{name}/mcp",
                self.release_namespace
            ),
            server_name: FERRISKEY_SERVER_NAME.to_string(),
            scopes_supported: Vec::new(),
            egress: Egress::Cluster,
        })
    }

    pub fn server_canonical_uri(&self, instance: &IdentityInstance, slug: &str) -> String {
        format!("{}/{slug}", self.canonical_uri(instance))
    }

    pub fn server_upstreams(&self, instance: &IdentityInstance, secret: &str) -> Vec<Upstream> {
        let Some(base) = self.upstream(instance, secret) else {
            return Vec::new();
        };
        let Some(agents) = instance.spec.agents.as_ref() else {
            return Vec::new();
        };

        servers::serving(agents)
            .into_iter()
            .filter(|server| servers::is_slug(&server.slug))
            .map(|server| Upstream {
                canonical_uri: self.server_canonical_uri(instance, &server.slug),
                audience: servers::audience(&server.slug),
                upstream_url: server.url.clone(),
                server_name: server.slug.clone(),
                egress: Egress::Public,
                ..base.clone()
            })
            .collect()
    }
}

fn quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn render(upstreams: &[Upstream]) -> String {
    let mut sorted: Vec<&Upstream> = upstreams.iter().collect();
    sorted.sort_by(|a, b| a.canonical_uri.cmp(&b.canonical_uri));

    let mut out = String::new();
    let _ = writeln!(out, "jwks_refresh_secs = {JWKS_REFRESH_SECS}");
    let _ = writeln!(out, "pdp_timeout_ms = {PDP_TIMEOUT_MS}");
    if sorted.is_empty() {
        out.push_str("upstreams = []\n");
        return out;
    }

    for upstream in sorted {
        let scopes = upstream
            .scopes_supported
            .iter()
            .map(|scope| quoted(scope))
            .collect::<Vec<_>>()
            .join(", ");
        let egress = match upstream.egress {
            Egress::Cluster => "",
            Egress::Public => "egress = \"public\"\n",
        };
        let _ = write!(
            out,
            "\n[[upstreams]]\ncanonical_uri = {}\nissuer = {}\njwks_uri = {}\nupstream_url = {}\nserver_name = {}\nscopes_supported = [{scopes}]\n{egress}\n[upstreams.pdp]\nevaluation_url = {}\nevaluations_url = {}\nclient_id = {}\nclient_secret = {}\n\n[upstreams.exchange]\ntoken_url = {}\nclient_id = {}\nclient_secret = {}\naudience = {}\n",
            quoted(&upstream.canonical_uri),
            quoted(&upstream.issuer),
            quoted(&upstream.jwks_uri),
            quoted(&upstream.upstream_url),
            quoted(&upstream.server_name),
            quoted(&upstream.evaluation_url),
            quoted(&upstream.evaluations_url),
            quoted(&upstream.client_id),
            quoted(&upstream.client_secret),
            quoted(&upstream.token_url),
            quoted(&upstream.client_id),
            quoted(&upstream.client_secret),
            quoted(&upstream.audience),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agents::test_spec;
    use autharie_crds::v1alpha::identity_instance::{AgentsSpec, IdentityInstanceSpec};

    fn instance(name: &str, namespace: &str, host: &str, realm: &str) -> IdentityInstance {
        let spec: IdentityInstanceSpec = test_spec(host);
        let mut instance = IdentityInstance::new(name, spec);
        instance.metadata.namespace = Some(namespace.to_string());
        instance.spec.agents = Some(AgentsSpec {
            enabled: true,
            realm: realm.to_string(),
            policies: Vec::new(),
            policies_revision: 0,
            servers: Vec::new(),
        });
        instance
    }

    fn environment() -> AgentsEnvironment {
        AgentsEnvironment::new("autharie-system", None)
    }

    fn upstream(host: &str) -> Upstream {
        environment()
            .upstream(
                &instance("deployment-1", "tenant-a", host, "acme"),
                "s3cret",
            )
            .expect("an upstream")
    }

    #[test]
    fn the_canonical_uri_is_the_mcp_path_of_the_public_host() {
        let instance = instance("deployment-1", "tenant-a", "auth.acme.test", "acme");

        assert_eq!(
            environment().canonical_uri(&instance),
            "https://auth.acme.test/mcp"
        );
        assert_eq!(
            AgentsEnvironment::new("autharie-system", Some(8443)).canonical_uri(&instance),
            "https://auth.acme.test:8443/mcp"
        );
    }

    #[test]
    fn an_upstream_names_the_public_issuer_and_the_in_cluster_endpoints() {
        let found = upstream("auth.acme.test");

        assert_eq!(found.canonical_uri, "https://auth.acme.test/mcp");
        assert_eq!(found.issuer, "https://auth.acme.test/api/realms/acme");
        assert_eq!(
            found.jwks_uri,
            "http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/protocol/openid-connect/certs"
        );
        assert_eq!(
            found.evaluation_url,
            "http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/authzen/access/v1/evaluation"
        );
        assert_eq!(
            found.evaluations_url,
            "http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/authzen/access/v1/evaluations"
        );
        assert_eq!(
            found.token_url,
            "http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/protocol/openid-connect/token"
        );
    }

    #[test]
    fn an_upstream_exchanges_for_the_realm_api_and_reaches_the_instance_mcp_server() {
        let found = upstream("auth.acme.test");

        assert_eq!(found.audience, "acme-realm");
        assert_eq!(found.client_id, "mcp-gateway");
        assert_eq!(found.client_secret, "s3cret");
        assert_eq!(found.server_name, "ferriskey");
        assert_eq!(
            found.upstream_url,
            "http://ferriskey-mcp.autharie-system.svc:8080/instances/tenant-a/deployment-1/mcp"
        );
    }

    #[test]
    fn a_pooled_cell_issues_under_its_own_host() {
        let mut pooled = instance("deployment-1", "tenant-a", "acme.zone.test", "acme");
        pooled.spec.ferriskey = Some(
            serde_json::from_value(serde_json::json!({ "pooledCell": true }))
                .expect("a ferriskey config"),
        );

        let found = environment().upstream(&pooled, "s").expect("an upstream");

        assert_eq!(found.issuer, "https://acme.zone.test/api/realms/acme");
    }

    #[test]
    fn an_instance_without_agents_has_no_upstream() {
        let mut plain = instance("deployment-1", "tenant-a", "auth.acme.test", "acme");
        plain.spec.agents = None;

        assert_eq!(environment().upstream(&plain, "s"), None);
    }

    #[test]
    fn no_upstream_renders_an_empty_list() {
        let text = render(&[]);

        assert!(text.contains("upstreams = []"));
        assert!(!text.contains("[[upstreams]]"));
    }

    #[test]
    fn the_gateway_settings_come_first_so_they_stay_top_level() {
        let text = render(&[upstream("auth.acme.test")]);

        let first_table = text.find("[[upstreams]]").expect("a table");
        assert!(text.find("jwks_refresh_secs = 300").expect("a setting") < first_table);
        assert!(text.find("pdp_timeout_ms = 2000").expect("a setting") < first_table);
    }

    #[test]
    fn an_upstream_renders_every_field_of_the_gateway_contract() {
        let text = render(&[upstream("auth.acme.test")]);

        for line in [
            "[[upstreams]]",
            "canonical_uri = \"https://auth.acme.test/mcp\"",
            "issuer = \"https://auth.acme.test/api/realms/acme\"",
            "upstream_url = \"http://ferriskey-mcp.autharie-system.svc:8080/instances/tenant-a/deployment-1/mcp\"",
            "server_name = \"ferriskey\"",
            "scopes_supported = []",
            "[upstreams.pdp]",
            "evaluation_url = \"http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/authzen/access/v1/evaluation\"",
            "evaluations_url = \"http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/authzen/access/v1/evaluations\"",
            "client_id = \"mcp-gateway\"",
            "client_secret = \"s3cret\"",
            "[upstreams.exchange]",
            "token_url = \"http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/protocol/openid-connect/token\"",
            "audience = \"acme-realm\"",
        ] {
            assert!(text.lines().any(|l| l == line), "{line}\n{text}");
        }
    }

    #[test]
    fn the_pdp_and_exchange_tables_follow_their_upstream() {
        let text = render(&[upstream("auth.acme.test")]);

        let upstream_at = text.find("[[upstreams]]").expect("table");
        let pdp_at = text.find("[upstreams.pdp]").expect("table");
        let exchange_at = text.find("[upstreams.exchange]").expect("table");
        assert!(upstream_at < pdp_at && pdp_at < exchange_at);
    }

    #[test]
    fn upstreams_are_rendered_in_a_stable_order_whatever_the_input_order() {
        let a = upstream("a.acme.test");
        let b = upstream("b.acme.test");

        assert_eq!(render(&[a.clone(), b.clone()]), render(&[b, a]));
    }

    #[test]
    fn two_upstreams_make_two_tables() {
        let text = render(&[upstream("a.acme.test"), upstream("b.acme.test")]);

        assert_eq!(text.matches("[[upstreams]]").count(), 2);
        assert!(text.find("a.acme.test/mcp").unwrap() < text.find("b.acme.test/mcp").unwrap());
    }

    #[test]
    fn strings_are_escaped() {
        let mut found = upstream("auth.acme.test");
        found.client_secret = "a\"b\\c\nd".to_string();

        let text = render(&[found]);

        assert!(text.contains("client_secret = \"a\\\"b\\\\c\\nd\""));
    }

    fn with_servers(servers: serde_json::Value) -> IdentityInstance {
        let mut found = instance("deployment-1", "tenant-a", "auth.acme.test", "acme");
        found.spec.agents.as_mut().expect("agents").servers =
            serde_json::from_value(servers).expect("servers");
        found
    }

    fn two_servers() -> IdentityInstance {
        with_servers(serde_json::json!([
            {
                "id": "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
                "slug": "crm",
                "url": "https://tools.example.com/mcp",
                "enabled": true,
            },
            {
                "id": "7f1c2d3e-0000-4000-8000-00000000000b",
                "slug": "erp",
                "url": "https://erp.example.com/mcp",
                "enabled": false,
            },
        ]))
    }

    #[test]
    fn a_customer_server_is_a_public_upstream_under_its_own_path_and_audience() {
        let found = environment().server_upstreams(&two_servers(), "s3cret");

        assert_eq!(found.len(), 1);
        let crm = &found[0];
        assert_eq!(crm.canonical_uri, "https://auth.acme.test/mcp/crm");
        assert_eq!(crm.server_name, "crm");
        assert_eq!(crm.audience, "mcp-crm");
        assert_eq!(crm.upstream_url, "https://tools.example.com/mcp");
        assert_eq!(crm.egress, Egress::Public);
    }

    #[test]
    fn a_customer_server_shares_the_issuer_and_the_decision_point_of_the_ferriskey_upstream() {
        let instance = two_servers();
        let base = environment()
            .upstream(&instance, "s3cret")
            .expect("upstream");

        let crm = environment()
            .server_upstreams(&instance, "s3cret")
            .remove(0);

        assert_eq!(crm.issuer, base.issuer);
        assert_eq!(crm.jwks_uri, base.jwks_uri);
        assert_eq!(crm.evaluation_url, base.evaluation_url);
        assert_eq!(crm.evaluations_url, base.evaluations_url);
        assert_eq!(crm.token_url, base.token_url);
        assert_eq!(crm.client_id, base.client_id);
        assert_eq!(crm.client_secret, base.client_secret);
        assert_eq!(base.egress, Egress::Cluster);
    }

    #[test]
    fn a_server_with_a_slug_nobody_may_use_gets_no_upstream() {
        let instance = with_servers(serde_json::json!([{
            "id": "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11",
            "slug": "gateway",
            "url": "https://tools.example.com/mcp",
            "enabled": true,
        }]));

        assert!(environment().server_upstreams(&instance, "s").is_empty());
    }

    #[test]
    fn a_public_upstream_says_so_and_a_cluster_one_stays_silent() {
        let instance = two_servers();
        let mut all = vec![upstream("auth.acme.test")];
        all.extend(environment().server_upstreams(&instance, "s3cret"));

        let text = render(&all);

        assert_eq!(text.matches("egress = \"public\"").count(), 1);
        assert!(
            text.contains("server_name = \"crm\"\nscopes_supported = []\negress = \"public\"\n")
        );
        assert!(text.contains("audience = \"mcp-crm\""));
    }

    #[test]
    fn the_rendered_document_is_one_the_gateway_reads_with_each_server_as_it_was_asked() {
        let instance = two_servers();
        let mut all = Vec::new();
        all.extend(environment().upstream(&instance, "s3cret"));
        all.extend(environment().server_upstreams(&instance, "s3cret"));

        let config = mcp_gateway::config::GatewayConfig::from_toml(&render(&all)).expect("parses");

        assert_eq!(config.upstreams.len(), 2);
        let ferriskey = config
            .upstreams
            .iter()
            .find(|u| u.server_name == "ferriskey")
            .expect("ferriskey");
        assert_eq!(ferriskey.egress, mcp_gateway::config::Egress::Cluster);
        let crm = config
            .upstreams
            .iter()
            .find(|u| u.server_name == "crm")
            .expect("crm");
        assert_eq!(crm.egress, mcp_gateway::config::Egress::Public);
        assert_eq!(crm.canonical_uri, "https://auth.acme.test/mcp/crm");
        assert_eq!(crm.exchange.audience, "mcp-crm");
        assert_eq!(crm.upstream_url, "https://tools.example.com/mcp");
        assert_eq!(crm.exchange.client_id, "mcp-gateway");
    }

    #[test]
    fn an_upstream_never_prints_its_secret() {
        let printed = format!("{:?}", upstream("auth.acme.test"));

        assert!(!printed.contains("s3cret"));
    }
}
