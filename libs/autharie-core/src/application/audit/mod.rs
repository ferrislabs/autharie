use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    audit::{
        AuditBatch, AuditEntry,
        commands::{ListAuditEntriesCommand, RecordAuditEntryCommand},
        ports::AuditService,
        service::AuditServiceImpl,
    },
};
use autharie_macros::transactional;

use crate::{AutharieService, infrastructure::role::permissions_in};

pub mod fleet;

impl AuditService for AutharieService {
    #[transactional(audit)]
    async fn record(&self, command: RecordAuditEntryCommand) -> Result<AuditEntry, CoreError> {
        AuditServiceImpl::new(audit_repository, permissions_in(&tx))
            .record(command)
            .await
    }

    #[transactional(audit)]
    async fn list_entries(
        &self,
        identity: Identity,
        command: ListAuditEntriesCommand,
    ) -> Result<AuditBatch, CoreError> {
        AuditServiceImpl::new(audit_repository, permissions_in(&tx))
            .list_entries(identity, command)
            .await
    }
}
