use autharie_core::{
    cloud_credentials::ProfileSpec,
    dataplane::{
        cloud_provider::{ControlPlaneOfferId, NodeType},
        cluster_profile::{ClusterMode, Replication},
    },
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::errors::ApiError;

#[derive(Deserialize, ToSchema)]
pub struct ClusterProfileRequest {
    pub mode: ClusterMode,
    pub control_plane_id: String,
    pub node_type: String,
    pub min_nodes: u8,
    pub max_nodes: u8,
    pub replication: u8,
}

impl ClusterProfileRequest {
    pub fn into_spec(self) -> Result<ProfileSpec, ApiError> {
        let replication =
            Replication::new(self.replication).ok_or_else(|| ApiError::BadRequest {
                reason: "replication must be at least 1".to_string(),
            })?;

        Ok(ProfileSpec {
            mode: self.mode,
            control_plane_id: ControlPlaneOfferId::new(self.control_plane_id),
            node_type: NodeType::new(self.node_type),
            min_nodes: self.min_nodes,
            max_nodes: self.max_nodes,
            replication,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(replication: u8) -> ClusterProfileRequest {
        ClusterProfileRequest {
            mode: ClusterMode::Standard,
            control_plane_id: "mutualized".to_string(),
            node_type: "small".to_string(),
            min_nodes: 2,
            max_nodes: 4,
            replication,
        }
    }

    #[test]
    fn zero_replicas_is_a_bad_request() {
        assert!(matches!(
            request(0).into_spec(),
            Err(ApiError::BadRequest { .. })
        ));
    }

    #[test]
    fn a_request_becomes_a_spec() {
        let spec = request(2).into_spec().expect("a spec");

        assert_eq!(spec.mode, ClusterMode::Standard);
        assert_eq!(spec.control_plane_id.as_str(), "mutualized");
        assert_eq!(spec.replication.get(), 2);
    }

    #[test]
    fn the_wire_names_are_snake_case() {
        let parsed: ClusterProfileRequest = serde_json::from_str(
            r#"{"mode":"ha","control_plane_id":"c","node_type":"n","min_nodes":3,"max_nodes":5,"replication":2}"#,
        )
        .expect("parsed");

        assert_eq!(parsed.mode, ClusterMode::Ha);
    }
}
