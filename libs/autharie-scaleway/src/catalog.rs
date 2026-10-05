use std::collections::HashMap;

use autharie_domain::dataplane::{
    cloud_provider::{
        CatalogError, ControlPlaneKind, ControlPlaneOffer, ControlPlaneOfferId, Money, NodeOffer,
        NodeType, Provider, ProviderCatalog, ProviderOffers,
    },
    credential::{CloudCredentialId, CloudCredentialStore, CredentialError},
    value_objects::Region,
};
use tracing::warn;

use crate::{
    config::ScalewayConfig,
    error::{ApiError, ScalewayError},
    http::{Http, Session},
    layout::Layout,
    models::{ClusterTypeList, Product, ProductList, ServerTypeList},
};

const HOURS_PER_MONTH: f64 = 720.0;
const CLUSTER_TYPE_SHORTAGE: &str = "shortage";
const PRODUCT_CATALOG_PATH: &str = "/product-catalog/v2alpha1/public-catalog/products";

pub struct ScalewayCatalog<S> {
    http: Http,
    config: ScalewayConfig,
    credentials: S,
}

impl<S: CloudCredentialStore> ScalewayCatalog<S> {
    pub fn new(config: ScalewayConfig, credentials: S) -> Result<Self, ScalewayError> {
        Ok(Self {
            http: Http::new(&config)?,
            config,
            credentials,
        })
    }

    async fn read(
        &self,
        session: &Session<'_>,
        region: &Region,
    ) -> Result<ProviderOffers, CatalogError> {
        let layout = Layout::new(&self.config, region.as_str());

        let types: ClusterTypeList = session
            .get(&layout.cluster_types(), &[])
            .await
            .map_err(|error| catalog_error(error, region))?;
        let servers: ServerTypeList = session
            .get(&layout.server_types(), &[])
            .await
            .map_err(|error| catalog_error(error, region))?;
        let prices = self.control_plane_prices(session, region).await;

        let control_planes = types
            .cluster_types
            .into_iter()
            .filter(|cluster_type| cluster_type.availability != CLUSTER_TYPE_SHORTAGE)
            .filter_map(|cluster_type| {
                let kind = if cluster_type.dedicated {
                    ControlPlaneKind::Dedicated
                } else {
                    ControlPlaneKind::Mutualized
                };
                let monthly_price = match (prices.get(&cluster_type.name), kind) {
                    (Some(price), _) => *price,
                    (None, ControlPlaneKind::Mutualized) => Money::ZERO,
                    (None, ControlPlaneKind::Dedicated) => {
                        warn!(
                            cluster_type = %cluster_type.name,
                            "no price found for a dedicated control plane, offer left out"
                        );
                        return None;
                    }
                };
                Some(ControlPlaneOffer {
                    id: ControlPlaneOfferId::new(cluster_type.name),
                    kind,
                    monthly_price,
                })
            })
            .collect();

        let mut node_types: Vec<NodeOffer> = servers
            .servers
            .into_iter()
            .filter(|(_, server)| server.ram >= self.config.min_node_memory_bytes)
            .filter_map(|(name, server)| {
                let monthly_price = to_minor_units(server.hourly_price * HOURS_PER_MONTH)?;
                Some(NodeOffer {
                    node_type: NodeType::new(name),
                    monthly_price: Money::new(monthly_price),
                })
            })
            .collect();
        node_types.sort_by(|a, b| a.node_type.as_str().cmp(b.node_type.as_str()));

        Ok(ProviderOffers {
            control_planes,
            node_types,
        })
    }

    async fn control_plane_prices(
        &self,
        session: &Session<'_>,
        region: &Region,
    ) -> HashMap<String, Money> {
        let query = [
            ("product_types", "kubernetes"),
            ("region", region.as_str()),
            ("page_size", "100"),
        ];
        match session
            .get::<ProductList>(PRODUCT_CATALOG_PATH, &query)
            .await
        {
            Ok(list) => list
                .products
                .iter()
                .filter_map(control_plane_price)
                .collect(),
            Err(error) => {
                warn!(%error, "the product catalog could not be read");
                HashMap::new()
            }
        }
    }
}

impl<S: CloudCredentialStore> ProviderCatalog for ScalewayCatalog<S> {
    async fn offers(
        &self,
        _provider: Provider,
        credential_id: &CloudCredentialId,
        region: &Region,
    ) -> Result<ProviderOffers, CatalogError> {
        let secret = self
            .credentials
            .get_for_provisioning(credential_id)
            .await
            .map_err(|error| match error {
                CredentialError::NotFound { .. } | CredentialError::Invalid => {
                    CatalogError::CredentialRejected
                }
                other => CatalogError::Unavailable(other.to_string()),
            })?;
        let session =
            Session::open(&self.http, &secret).map_err(|_| CatalogError::CredentialRejected)?;
        self.read(&session, region).await
    }
}

fn catalog_error(error: ApiError, region: &Region) -> CatalogError {
    if error.is_denied() {
        CatalogError::CredentialRejected
    } else if error.mentions_location() || error.has_kind("out_of_stock") {
        CatalogError::RegionUnavailable {
            region: region.as_str().to_string(),
        }
    } else {
        CatalogError::Unavailable(error.to_string())
    }
}

fn control_plane_price(product: &Product) -> Option<(String, Money)> {
    product
        .properties
        .as_ref()?
        .kubernetes
        .as_ref()?
        .kapsule_control_plane
        .as_ref()?;
    let retail = product.price.as_ref()?.retail_price.as_ref()?;
    let measure = product.unit_of_measure.as_ref()?;
    let amount = retail.units as f64 + f64::from(retail.nanos) / 1_000_000_000.0;
    let per_unit = amount / measure.size.max(1) as f64;
    let monthly = match measure.unit.as_str() {
        "hour" => per_unit * HOURS_PER_MONTH,
        "month" => per_unit,
        _ => return None,
    };
    Some((
        product.variant.clone(),
        Money::new(to_minor_units(monthly)?),
    ))
}

fn to_minor_units(amount: f64) -> Option<u64> {
    (amount.is_finite() && amount >= 0.0).then(|| (amount * 100.0).round() as u64)
}
