use autharie_auth::Identity;
use autharie_domain::{
    CoreError,
    traces::{
        TraceDetail, TraceSearchResult,
        commands::{ReadTraceCommand, SearchTracesCommand},
        ports::TraceSearchIndex,
        service::TraceSearchServiceImpl,
    },
};
use autharie_macros::transactional;

use crate::{AutharieService, infrastructure::role::permissions_in, policy::AuthariePolicy};

impl AutharieService {
    /// Runs a trace search, gated the same way [`super::logs::search_logs`]
    /// gates a log search: permission first, tenant ownership of any
    /// deployment filter second, both before `search_index` -- the caller's
    /// own Quickwit adapter, held outside this crate for the same reason the
    /// logs one is -- is ever asked anything.
    #[transactional(deployment, audit, user)]
    pub async fn search_traces<S>(
        &self,
        identity: Identity,
        command: SearchTracesCommand,
        search_index: S,
    ) -> Result<TraceSearchResult, CoreError>
    where
        S: TraceSearchIndex,
    {
        TraceSearchServiceImpl::new(
            deployment_repository,
            audit_repository,
            user_repository,
            AuthariePolicy::new(permissions_in(&tx)),
            search_index,
        )
        .search(identity, command)
        .await
    }

    /// Every span of one trace, for the waterfall view.
    #[transactional(deployment, audit, user)]
    pub async fn get_trace<S>(
        &self,
        identity: Identity,
        command: ReadTraceCommand,
        search_index: S,
    ) -> Result<TraceDetail, CoreError>
    where
        S: TraceSearchIndex,
    {
        TraceSearchServiceImpl::new(
            deployment_repository,
            audit_repository,
            user_repository,
            AuthariePolicy::new(permissions_in(&tx)),
            search_index,
        )
        .trace(identity, command)
        .await
    }
}
