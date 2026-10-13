use kube::api::{DeleteParams, DynamicObject, Patch, PatchParams};
use kube::core::{ApiResource, GroupVersionKind};
use kube::{Api, Client};
use serde_json::{Value, json};

use std::collections::BTreeMap;

use crate::domain::entities::realm_payload::{RouteOwner, TenantRoute};
use crate::domain::error::GenesisError;
use crate::domain::ports::TenantRoutePort;

const FIELD_MANAGER: &str = "genesis";
const GROUP: &str = "gateway.networking.k8s.io";
const VERSION: &str = "v1";
const NOT_FOUND: u16 = 404;
const CELL_LABEL: &str = "autharie.fr/cell";
const DEPLOYMENT_LABEL: &str = "autharie.fr/deployment";
const REALM_LABEL: &str = "autharie.fr/realm";

fn httproutes(client: Client, namespace: &str) -> Api<DynamicObject> {
    let resource = ApiResource::from_gvk(&GroupVersionKind::gvk(GROUP, VERSION, "HTTPRoute"));
    Api::namespaced_with(client, namespace, &resource)
}

pub fn tenant_route(
    cell_route: &Value,
    route: &TenantRoute,
    hostname: &str,
) -> Result<Value, GenesisError> {
    let spec = &cell_route["spec"];
    let present = |value: &Value| value.as_array().is_some_and(|items| !items.is_empty());
    if !present(&spec["parentRefs"]) || !present(&spec["rules"]) {
        return Err(GenesisError::Kubernetes {
            message: format!(
                "the route of cell {} has no gateway or no backend yet",
                route.cell.name
            ),
        });
    }

    let mut metadata = json!({
        "name": route.name,
        "namespace": route.cell.namespace,
        "labels": {
            "app.kubernetes.io/managed-by": FIELD_MANAGER,
            CELL_LABEL: route.cell.name,
            DEPLOYMENT_LABEL: route.deployment_id.to_string(),
            REALM_LABEL: route.realm.as_str(),
        },
    });
    if let Some(owners) = cell_route["metadata"].get("ownerReferences") {
        metadata["ownerReferences"] = owners.clone();
    }

    Ok(json!({
        "apiVersion": format!("{GROUP}/{VERSION}"),
        "kind": "HTTPRoute",
        "metadata": metadata,
        "spec": {
            "parentRefs": spec["parentRefs"],
            "hostnames": [hostname],
            "rules": spec["rules"],
        },
    }))
}

pub fn route_owner(labels: Option<&BTreeMap<String, String>>) -> RouteOwner {
    let label = |key: &str| labels.and_then(|labels| labels.get(key)).cloned();
    RouteOwner {
        deployment: label(DEPLOYMENT_LABEL),
        realm: label(REALM_LABEL),
    }
}

pub struct KubeTenantRoutes {
    client: Client,
}

impl KubeTenantRoutes {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

fn kube_error(error: kube::Error) -> GenesisError {
    GenesisError::Kubernetes {
        message: error.to_string(),
    }
}

impl TenantRoutePort for KubeTenantRoutes {
    async fn apply(&self, route: &TenantRoute, hostname: &str) -> Result<(), GenesisError> {
        let routes = httproutes(self.client.clone(), &route.cell.namespace);
        let cell_route = routes
            .get_opt(&route.cell.name)
            .await
            .map_err(kube_error)?
            .ok_or_else(|| GenesisError::Kubernetes {
                message: format!("cell {} has no route yet", route.cell.name),
            })?;
        let cell_route =
            serde_json::to_value(&cell_route).map_err(|error| GenesisError::Internal {
                message: error.to_string(),
            })?;

        let desired = tenant_route(&cell_route, route, hostname)?;
        routes
            .patch(
                &route.name,
                &PatchParams::apply(FIELD_MANAGER).force(),
                &Patch::Apply(&desired),
            )
            .await
            .map_err(kube_error)?;
        Ok(())
    }

    async fn owner(&self, route: &TenantRoute) -> Result<Option<RouteOwner>, GenesisError> {
        let found = httproutes(self.client.clone(), &route.cell.namespace)
            .get_opt(&route.name)
            .await
            .map_err(kube_error)?;
        Ok(found.map(|found| route_owner(found.metadata.labels.as_ref())))
    }

    async fn delete(&self, route: &TenantRoute) -> Result<(), GenesisError> {
        let routes = httproutes(self.client.clone(), &route.cell.namespace);
        match routes.delete(&route.name, &DeleteParams::default()).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(response)) if response.code == NOT_FOUND => Ok(()),
            Err(error) => Err(kube_error(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::entities::identity_instance::IdentityInstanceRef;
    use crate::domain::entities::realm_payload::RealmName;

    fn route() -> TenantRoute {
        TenantRoute {
            name: format!("realm-{}", Uuid::from_u128(1)),
            cell: IdentityInstanceRef::for_deployment(Uuid::from_u128(2), "cells-one"),
            deployment_id: Uuid::from_u128(1),
            realm: RealmName::try_from("acme").expect("valid"),
        }
    }

    fn cell_route() -> Value {
        json!({
            "apiVersion": "gateway.networking.k8s.io/v1",
            "kind": "HTTPRoute",
            "metadata": {
                "name": route().cell.name,
                "namespace": "cells-one",
                "labels": { "app.kubernetes.io/component": "route" },
                "ownerReferences": [{
                    "apiVersion": "autharie.fr/v1alpha",
                    "kind": "IdentityInstance",
                    "name": route().cell.name,
                    "uid": "0000-uid",
                }],
            },
            "spec": {
                "parentRefs": [{
                    "group": "gateway.networking.k8s.io",
                    "kind": "Gateway",
                    "name": "autharie-dataplane",
                    "namespace": "autharie-system",
                }],
                "hostnames": ["cell-one.cells.example"],
                "rules": [
                    {
                        "matches": [{ "path": { "type": "PathPrefix", "value": "/api" } }],
                        "backendRefs": [{ "name": "cell-api", "port": 3333 }],
                    },
                    {
                        "matches": [{ "path": { "type": "PathPrefix", "value": "/" } }],
                        "backendRefs": [{ "name": "cell-webapp", "port": 80 }],
                    },
                ],
            },
        })
    }

    #[test]
    fn a_tenant_route_reuses_the_gateway_and_backends_of_its_cell_for_its_own_hostname() {
        let cell = cell_route();

        let built = tenant_route(&cell, &route(), "acme.cells.example").expect("built");

        assert_eq!(built["metadata"]["name"], json!(route().name));
        assert_eq!(built["metadata"]["namespace"], json!("cells-one"));
        assert_eq!(built["spec"]["hostnames"], json!(["acme.cells.example"]));
        assert_eq!(built["spec"]["parentRefs"], cell["spec"]["parentRefs"]);
        assert_eq!(built["spec"]["rules"], cell["spec"]["rules"]);
        assert_eq!(
            built["metadata"]["ownerReferences"],
            cell["metadata"]["ownerReferences"]
        );
    }

    #[test]
    fn a_tenant_route_is_labelled_with_its_deployment_and_realm_and_read_back() {
        let built = tenant_route(&cell_route(), &route(), "acme.cells.example").expect("built");
        let labels: BTreeMap<String, String> =
            serde_json::from_value(built["metadata"]["labels"].clone()).expect("labels");

        let owner = route_owner(Some(&labels));

        assert_eq!(
            owner,
            RouteOwner {
                deployment: Some(Uuid::from_u128(1).to_string()),
                realm: Some("acme".to_string()),
            }
        );
        assert!(route().is_owned_by(&owner));
        assert_eq!(route_owner(None), RouteOwner::default());
    }

    #[test]
    fn a_cell_route_without_a_gateway_or_a_backend_is_not_copied() {
        for missing in ["parentRefs", "rules"] {
            let mut cell = cell_route();
            cell["spec"][missing] = json!([]);

            let result = tenant_route(&cell, &route(), "acme.cells.example");

            assert!(
                matches!(result, Err(GenesisError::Kubernetes { .. })),
                "{missing}"
            );
        }
    }

    #[test]
    fn a_cell_route_without_owner_gives_a_route_without_owner() {
        let mut cell = cell_route();
        cell["metadata"]
            .as_object_mut()
            .expect("object")
            .remove("ownerReferences");

        let built = tenant_route(&cell, &route(), "acme.cells.example").expect("built");

        assert!(built["metadata"].get("ownerReferences").is_none());
    }
}
