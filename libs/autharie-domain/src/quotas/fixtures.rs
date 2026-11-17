use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{
    CoreError,
    organisation::OrganisationId,
    quotas::{
        Quota, QuotaKind,
        ports::{QuotaChange, QuotaRepository},
    },
};

/// Overrides held in memory, for the tests of every service that checks one.
#[derive(Clone, Default)]
pub struct InMemoryQuotas {
    overrides: Arc<Mutex<HashMap<(OrganisationId, QuotaKind), Quota>>>,
}

impl InMemoryQuotas {
    pub fn with(self, organisation_id: OrganisationId, kind: QuotaKind, quota: Quota) -> Self {
        self.overrides
            .lock()
            .unwrap()
            .insert((organisation_id, kind), quota);
        self
    }
}

impl QuotaRepository for InMemoryQuotas {
    async fn overrides(
        &self,
        organisation_id: OrganisationId,
    ) -> Result<HashMap<QuotaKind, Quota>, CoreError> {
        Ok(self
            .overrides
            .lock()
            .unwrap()
            .iter()
            .filter(|((organisation, _), _)| *organisation == organisation_id)
            .map(|((_, kind), quota)| (*kind, *quota))
            .collect())
    }

    async fn set_override(
        &self,
        organisation_id: OrganisationId,
        kind: QuotaKind,
        quota: Option<Quota>,
        _change: &QuotaChange,
    ) -> Result<(), CoreError> {
        let mut overrides = self.overrides.lock().unwrap();
        match quota {
            Some(quota) => overrides.insert((organisation_id, kind), quota),
            None => overrides.remove(&(organisation_id, kind)),
        };
        Ok(())
    }
}
