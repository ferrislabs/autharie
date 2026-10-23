use std::time::Duration;

use autharie_crds::v1alpha::identity_instance::{
    AgentServerPhase, AgentServerSpec, AgentServerStatus, AgentsSpec, IdentityInstance,
};
use chrono::{DateTime, Utc};
use futures::future::join_all;

use super::{AgentsReconciler, Tally};
use crate::domain::agents::ports::{
    AgentInstances, AgentsEdge, AgentsSecretsPort, AgentsStatusRepository, FerriskeyAgentsPort,
    GatewayConfigStore,
};
use crate::domain::agents::servers::{
    self, AgentServersPort, Discovered, DiscoveryTarget, Due, REDISCOVER_AFTER, RETRY_FAILED_AFTER,
};
use crate::domain::agents::{AgentsError, settings_without_resources};

struct Synced {
    entry: AgentServerStatus,
    issue: Option<AgentsError>,
    retry: bool,
    revisit: Option<Duration>,
}

impl Synced {
    fn kept(entry: AgentServerStatus) -> Self {
        Self {
            entry,
            issue: None,
            retry: false,
            revisit: None,
        }
    }
}

impl<F, S, R, L, G, E, V> AgentsReconciler<F, S, R, L, G, E, V>
where
    F: FerriskeyAgentsPort,
    S: AgentsSecretsPort,
    R: AgentsStatusRepository,
    L: AgentInstances,
    G: GatewayConfigStore,
    E: AgentsEdge,
    V: AgentServersPort,
{
    pub(super) fn server_resources(
        &self,
        instance: &IdentityInstance,
        agents: &AgentsSpec,
    ) -> Vec<String> {
        servers::serving(agents)
            .into_iter()
            .filter(|server| servers::is_slug(&server.slug))
            .map(|server| {
                self.environment
                    .server_canonical_uri(instance, &server.slug)
            })
            .collect()
    }

    pub(super) fn doomed_servers(
        &self,
        instance: &IdentityInstance,
        agents: &AgentsSpec,
    ) -> Vec<String> {
        let mut slugs: Vec<String> = agents
            .servers
            .iter()
            .map(|server| server.slug.clone())
            .chain(
                servers::known(instance)
                    .iter()
                    .map(|known| known.slug.clone()),
            )
            .filter(|slug| servers::is_slug(slug))
            .collect();
        slugs.sort();
        slugs.dedup();
        slugs
    }

    pub(super) async fn sync_servers(
        &self,
        instance: &IdentityInstance,
        agents: &AgentsSpec,
        client_ready: bool,
        tally: &mut Tally,
    ) -> Vec<AgentServerStatus> {
        let previous = servers::known(instance);
        let wanted = servers::serving(agents);
        let still_wanted = |known: &AgentServerStatus| {
            wanted
                .iter()
                .any(|server| server.id == known.id && server.slug == known.slug)
        };

        if !client_ready {
            return previous
                .iter()
                .filter(|known| still_wanted(known))
                .cloned()
                .collect();
        }

        let realm = agents.realm.as_str();
        let mut entries = Vec::with_capacity(wanted.len());
        let mut taken_back = Vec::new();
        for gone in previous.iter().filter(|known| !still_wanted(known)) {
            if !servers::is_slug(&gone.slug) {
                continue;
            }
            let uri = self.environment.server_canonical_uri(instance, &gone.slug);
            match self
                .servers
                .remove_server_client(instance, realm, &gone.slug, &uri)
                .await
            {
                Ok(()) | Err(AgentsError::NotFound) => taken_back.push(uri),
                Err(error) => {
                    tally.issue(error);
                    entries.push(gone.clone());
                }
            }
        }
        if !taken_back.is_empty() {
            self.forget_resources(instance, realm, &taken_back, tally)
                .await;
        }

        let now = (self.clock)();
        let synced = join_all(
            wanted
                .iter()
                .map(|server| self.sync_one(instance, realm, server, previous, now)),
        )
        .await;
        for done in synced {
            if let Some(issue) = done.issue {
                tally.issue(issue);
            }
            tally.retry |= done.retry;
            if let Some(wait) = done.revisit {
                tally.revisit_in(wait);
            }
            entries.push(done.entry);
        }
        entries
    }

    async fn forget_resources(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        uris: &[String],
        tally: &mut Tally,
    ) {
        match self.ferriskey.realm_settings(instance, realm).await {
            Ok(current) => {
                if let Some(wanted) = settings_without_resources(&current, uris)
                    && let Err(error) = self
                        .ferriskey
                        .update_realm_settings(instance, realm, &wanted)
                        .await
                {
                    tally.issue(error);
                }
            }
            Err(error) => tally.issue(error),
        }
    }

    async fn sync_one(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        spec: &AgentServerSpec,
        previous: &[AgentServerStatus],
        now: DateTime<Utc>,
    ) -> Synced {
        let last = previous.iter().find(|known| known.id == spec.id);
        let keep = || {
            last.cloned()
                .unwrap_or_else(|| servers::waiting(spec, "waiting for the first discovery"))
        };

        if !servers::is_slug(&spec.slug) {
            return Synced::kept(servers::entry(
                spec,
                Discovered::failed(AgentServerPhase::Invalid, "the slug is not valid"),
                now,
            ));
        }

        let uri = self.environment.server_canonical_uri(instance, &spec.slug);
        if let Err(error) = self
            .servers
            .ensure_server_client(instance, realm, &spec.slug, &uri)
            .await
        {
            return Synced {
                entry: keep(),
                issue: Some(error),
                retry: true,
                revisit: None,
            };
        }

        if let Due::After(wait) = servers::discovery_due(spec, last, now) {
            return Synced {
                revisit: Some(wait),
                ..Synced::kept(keep())
            };
        }

        let secret = match self.secrets.load(instance).await {
            Ok(Some(secret)) => secret,
            Ok(None) => {
                return Synced {
                    retry: true,
                    ..Synced::kept(keep())
                };
            }
            Err(error) => {
                return Synced {
                    entry: keep(),
                    issue: Some(error),
                    retry: true,
                    revisit: None,
                };
            }
        };
        let Some(upstream) = self.environment.upstream(instance, &secret) else {
            return Synced::kept(keep());
        };

        let audience = servers::audience(&spec.slug);
        let found = self
            .servers
            .discover(&DiscoveryTarget {
                token_url: &upstream.token_url,
                client_id: &upstream.client_id,
                client_secret: &upstream.client_secret,
                audience: &audience,
                url: &spec.url,
            })
            .await;
        let found = Discovered {
            protocol_version: found
                .protocol_version
                .or_else(|| last.and_then(|known| known.protocol_version.clone())),
            ..found
        };
        let wait = if found.phase == AgentServerPhase::Ready {
            REDISCOVER_AFTER
        } else {
            RETRY_FAILED_AFTER
        };

        Synced {
            entry: servers::entry(spec, found, now),
            issue: None,
            retry: false,
            revisit: Some(wait),
        }
    }
}
