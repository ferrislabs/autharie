use axum::{Router, middleware::from_fn_with_state};
use axum_extra::routing::RouterExt;
use utoipa::OpenApi;

use crate::{
    handlers::deployments::{
        agent_registry::{
            __path_approve_agent_handler, __path_block_agent_handler,
            __path_create_autonomous_agent_handler, __path_delete_agent_handler,
            __path_get_agent_credential_handler, __path_list_agent_clients_handler,
            __path_put_agent_access_handler, __path_rotate_autonomous_secret_handler,
            __path_unblock_agent_handler, approve_agent_handler, block_agent_handler,
            create_autonomous_agent_handler, delete_agent_handler, get_agent_credential_handler,
            list_agent_clients_handler, put_agent_access_handler, rotate_autonomous_secret_handler,
            unblock_agent_handler,
        },
        agent_servers::{
            __path_discover_agent_server_handler, __path_list_agent_servers_handler,
            __path_register_agent_server_handler, __path_remove_agent_server_handler,
            __path_update_agent_server_handler, discover_agent_server_handler,
            list_agent_servers_handler, register_agent_server_handler, remove_agent_server_handler,
            update_agent_server_handler,
        },
        agents::{
            __path_create_agent_policy_handler, __path_delete_agent_policy_handler,
            __path_get_agents_handler, __path_list_agent_decisions_handler,
            __path_list_agent_policies_handler, __path_put_agents_handler,
            __path_update_agent_policy_handler, create_agent_policy_handler,
            delete_agent_policy_handler, get_agents_handler, list_agent_decisions_handler,
            list_agent_policies_handler, put_agents_handler, update_agent_policy_handler,
        },
        availability::{
            __path_get_deployment_availability_handler, get_deployment_availability_handler,
        },
        backups::{
            __path_ask_for_backup_handler, __path_get_backup_schedule_handler,
            __path_list_backups_handler, __path_restore_backup_handler,
            __path_set_backup_schedule_handler, ask_for_backup_handler,
            get_backup_schedule_handler, list_backups_handler, restore_backup_handler,
            set_backup_schedule_handler,
        },
        create_deployment::{__path_create_deployment_handler, create_deployment_handler},
        cutover::{__path_cutover_handler, cutover_handler},
        delete_deployment::{__path_delete_deployment_handler, delete_deployment_handler},
        features::{__path_get_deployment_features_handler, get_deployment_features_handler},
        get_deployment::{__path_get_deployment_handler, get_deployment_handler},
        iam_settings::{
            __path_get_iam_settings_handler, __path_set_iam_settings_handler,
            get_iam_settings_handler, set_iam_settings_handler,
        },
        list_deployments::{__path_list_deployments_handler, list_deployments_handler},
        network_access::{
            __path_get_network_access_handler, __path_set_network_access_handler,
            get_network_access_handler, set_network_access_handler,
        },
        power::{
            __path_start_deployment_handler, __path_stop_deployment_handler,
            start_deployment_handler, stop_deployment_handler,
        },
        read_logs::{__path_read_logs_handler, read_logs_handler},
        resize_cluster::{__path_resize_cluster_handler, resize_cluster_handler},
        smtp::{
            __path_delete_smtp_handler, __path_get_smtp_handler,
            __path_list_branding_realms_handler, __path_put_smtp_handler, delete_smtp_handler,
            get_smtp_handler, list_branding_realms_handler, put_smtp_handler,
        },
        subscription::{
            __path_get_subscription_handler, __path_put_subscription_handler,
            get_subscription_handler, put_subscription_handler,
        },
        update_deployment::{__path_update_deployment_handler, update_deployment_handler},
        upgrade_deployment::{__path_upgrade_deployment_handler, upgrade_deployment_handler},
        upgrade_in_flight::{__path_upgrade_in_flight_handler, upgrade_in_flight_handler},
        upgrade_settings::{__path_set_upgrade_settings_handler, set_upgrade_settings_handler},
        webhooks::{
            __path_create_webhook_handler, __path_delete_webhook_handler,
            __path_get_webhook_handler, __path_list_webhook_deliveries_handler,
            __path_list_webhook_realms_handler, __path_list_webhooks_handler,
            __path_retry_webhook_delivery_handler, __path_reveal_webhook_secret_handler,
            __path_rotate_webhook_secret_handler, __path_update_webhook_handler,
            create_webhook_handler, delete_webhook_handler, get_webhook_handler,
            list_webhook_deliveries_handler, list_webhook_events_handler,
            list_webhook_realms_handler, list_webhooks_handler, retry_webhook_delivery_handler,
            reveal_webhook_secret_handler, rotate_webhook_secret_handler, update_webhook_handler,
        },
    },
    router::service_auth_middleware,
    state::AppState,
};

pub mod agent_registry;
pub mod agent_servers;
pub mod agents;
pub mod availability;
pub mod backups;
pub mod create_deployment;
pub mod cutover;
pub mod delete_deployment;
pub mod deployment_view;
pub mod features;
pub mod get_deployment;
pub mod iam_settings;
pub mod list_deployments;
pub mod network_access;
pub mod power;
pub mod read_logs;
pub mod resize_cluster;
pub mod smtp;
pub mod subscription;
pub mod update_deployment;
pub mod upgrade_deployment;
pub mod upgrade_in_flight;
pub mod upgrade_settings;
pub mod webhooks;

#[derive(OpenApi)]
#[openapi(
    paths(
        list_deployments_handler,
        create_deployment_handler,
        get_deployment_handler,
        update_deployment_handler,
        delete_deployment_handler,
        stop_deployment_handler,
        start_deployment_handler,
        upgrade_deployment_handler,
        upgrade_in_flight_handler,
        read_logs_handler,
        set_upgrade_settings_handler,
        get_network_access_handler,
        set_network_access_handler,
        get_deployment_availability_handler,
        get_iam_settings_handler,
        set_iam_settings_handler,
        list_backups_handler,
        get_backup_schedule_handler,
        set_backup_schedule_handler,
        restore_backup_handler,
        ask_for_backup_handler,
        cutover_handler,
        resize_cluster_handler,
        get_subscription_handler,
        put_subscription_handler,
        get_deployment_features_handler,
        list_webhooks_handler,
        create_webhook_handler,
        get_webhook_handler,
        update_webhook_handler,
        delete_webhook_handler,
        rotate_webhook_secret_handler,
        reveal_webhook_secret_handler,
        list_webhook_deliveries_handler,
        retry_webhook_delivery_handler,
        list_webhook_realms_handler,
        get_agents_handler,
        put_agents_handler,
        list_agent_policies_handler,
        create_agent_policy_handler,
        update_agent_policy_handler,
        delete_agent_policy_handler,
        list_agent_decisions_handler,
        list_agent_clients_handler,
        put_agent_access_handler,
        approve_agent_handler,
        block_agent_handler,
        unblock_agent_handler,
        delete_agent_handler,
        create_autonomous_agent_handler,
        rotate_autonomous_secret_handler,
        get_agent_credential_handler,
        list_agent_servers_handler,
        register_agent_server_handler,
        update_agent_server_handler,
        remove_agent_server_handler,
        discover_agent_server_handler,
        get_smtp_handler,
        put_smtp_handler,
        delete_smtp_handler,
        list_branding_realms_handler,
    ),
    tags(
        (name = "deployments", description = "Deployment management endpoints scoped to organisations.")
    )
)]
pub struct DeploymentApiDoc;

pub fn deployment_routes(app_state: AppState) -> Router<AppState> {
    Router::new()
        .typed_get(list_deployments_handler)
        .typed_post(create_deployment_handler)
        .typed_post(upgrade_deployment_handler)
        .typed_post(stop_deployment_handler)
        .typed_post(start_deployment_handler)
        .typed_get(upgrade_in_flight_handler)
        .typed_get(read_logs_handler)
        .typed_put(set_upgrade_settings_handler)
        .typed_get(get_network_access_handler)
        .typed_get(get_deployment_availability_handler)
        .typed_put(set_network_access_handler)
        .typed_get(get_iam_settings_handler)
        .typed_put(set_iam_settings_handler)
        .typed_get(list_backups_handler)
        .typed_get(get_backup_schedule_handler)
        .typed_put(set_backup_schedule_handler)
        .typed_post(restore_backup_handler)
        .typed_post(ask_for_backup_handler)
        .typed_post(cutover_handler)
        .typed_put(resize_cluster_handler)
        .typed_get(get_subscription_handler)
        .typed_put(put_subscription_handler)
        .typed_get(get_deployment_features_handler)
        .typed_get(list_webhooks_handler)
        .typed_post(create_webhook_handler)
        .typed_get(get_webhook_handler)
        .typed_put(update_webhook_handler)
        .typed_delete(delete_webhook_handler)
        .typed_post(rotate_webhook_secret_handler)
        .typed_get(reveal_webhook_secret_handler)
        .typed_get(list_webhook_deliveries_handler)
        .typed_post(retry_webhook_delivery_handler)
        .typed_get(list_webhook_realms_handler)
        .typed_get(list_webhook_events_handler)
        .typed_get(get_agents_handler)
        .typed_put(put_agents_handler)
        .typed_get(list_agent_policies_handler)
        .typed_post(create_agent_policy_handler)
        .typed_put(update_agent_policy_handler)
        .typed_delete(delete_agent_policy_handler)
        .typed_get(list_agent_decisions_handler)
        .typed_get(list_agent_clients_handler)
        .typed_put(put_agent_access_handler)
        .typed_post(approve_agent_handler)
        .typed_post(block_agent_handler)
        .typed_post(unblock_agent_handler)
        .typed_delete(delete_agent_handler)
        .typed_post(create_autonomous_agent_handler)
        .typed_post(rotate_autonomous_secret_handler)
        .typed_get(get_agent_credential_handler)
        .typed_get(list_agent_servers_handler)
        .typed_post(register_agent_server_handler)
        .typed_put(update_agent_server_handler)
        .typed_delete(remove_agent_server_handler)
        .typed_post(discover_agent_server_handler)
        .typed_get(get_smtp_handler)
        .typed_put(put_smtp_handler)
        .typed_delete(delete_smtp_handler)
        .typed_get(list_branding_realms_handler)
        .typed_get(get_deployment_handler)
        .typed_patch(update_deployment_handler)
        .typed_delete(delete_deployment_handler)
        .layer(from_fn_with_state(
            app_state.clone(),
            service_auth_middleware,
        ))
}

#[cfg(test)]
mod tests {
    use super::deployment_routes;
    use crate::test_helpers::app_state;

    #[tokio::test]
    async fn deployment_routes_builds() {
        let state = app_state();
        let _router = deployment_routes(state);
    }
}
