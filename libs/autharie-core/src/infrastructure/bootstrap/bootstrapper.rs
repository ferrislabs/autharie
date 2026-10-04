use autharie_domain::{
    CoreError,
    dataplane::{
        bootstrap::{BootstrapRequest, ClusterBootstrapper},
        herald_identity::{HeraldBinding, MintedHeraldIdentity},
        ports::HeraldIdentityProvisioner,
        value_objects::DataPlaneId,
    },
};
use serde_json::{Value, json};
use tracing::{info, warn};

use super::{
    config::HelmConfig,
    runner::{HelmOutcome, HelmRunner, TokioHelmRunner},
    workdir::Workdir,
};

const RELEASE: &str = "autharie-dataplane";
const NAMESPACE: &str = "autharie";
const KUBECONFIG_FILE: &str = "kubeconfig";
const VALUES_FILE: &str = "values.json";
const MIN_REDACTED_PART: usize = 8;

pub struct HelmBootstrapper<I, R = TokioHelmRunner> {
    identities: I,
    runner: R,
    config: HelmConfig,
}

impl<I: HeraldIdentityProvisioner> HelmBootstrapper<I> {
    pub fn new(identities: I, config: HelmConfig) -> Self {
        let runner = TokioHelmRunner::new(config.helm_binary.clone(), config.timeout);
        Self {
            identities,
            runner,
            config,
        }
    }
}

impl<I: HeraldIdentityProvisioner, R: HelmRunner> HelmBootstrapper<I, R> {
    pub fn with_runner(identities: I, runner: R, config: HelmConfig) -> Self {
        Self {
            identities,
            runner,
            config,
        }
    }

    async fn install(
        &self,
        request: &BootstrapRequest,
        minted: &MintedHeraldIdentity,
    ) -> Result<(), CoreError> {
        let workdir = Workdir::create()?;
        let kubeconfig =
            workdir.write_private(KUBECONFIG_FILE, request.kubeconfig.expose().as_bytes())?;
        let values = workdir.write_private(
            VALUES_FILE,
            values(&self.config, request.data_plane_id, minted)
                .to_string()
                .as_bytes(),
        )?;

        let args = helm_args(&self.config, &kubeconfig, &values);
        let outcome = self.runner.run(&args, workdir.path()).await?;

        if outcome.succeeded() {
            return Ok(());
        }

        Err(CoreError::InternalError(format!(
            "installing the data plane chart failed (exit {}): {}",
            outcome
                .code
                .map_or_else(|| "signal".to_string(), |c| c.to_string()),
            redact(&outcome, &minted.secret, request.kubeconfig.expose()),
        )))
    }
}

pub(crate) fn values(config: &HelmConfig, id: DataPlaneId, minted: &MintedHeraldIdentity) -> Value {
    let mut values = json!({
        "dataplane": { "id": id.0.to_string() },
        "controlPlane": {
            "url": config.control_plane_url,
            "auth": {
                "issuer": config.herald_issuer,
                "clientId": minted.binding.client_id,
                "clientSecret": minted.secret,
            },
        },
    });

    if let Some(registry) = &config.image_registry {
        values["image"]["registry"] = json!(registry);
    }
    if let Some(tag) = &config.image_tag {
        values["image"]["tag"] = json!(tag);
    }

    values
}

pub(crate) fn helm_args(
    config: &HelmConfig,
    kubeconfig: &std::path::Path,
    values: &std::path::Path,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "upgrade",
        "--install",
        RELEASE,
        config.chart.as_str(),
        "--namespace",
        NAMESPACE,
        "--create-namespace",
        "--wait",
        "--atomic",
        "--timeout",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    args.push(format!("{}s", config.timeout.as_secs()));
    args.push("--kubeconfig".to_string());
    args.push(kubeconfig.to_string_lossy().into_owned());
    args.push("--values".to_string());
    args.push(values.to_string_lossy().into_owned());

    if let Some(version) = &config.chart_version {
        args.push("--version".to_string());
        args.push(version.clone());
    }

    args
}

fn redact(outcome: &HelmOutcome, secret: &str, kubeconfig: &str) -> String {
    let line = &outcome.last_line;
    let leaks = (!secret.is_empty() && line.contains(secret))
        || (!kubeconfig.is_empty() && line.contains(kubeconfig))
        || kubeconfig
            .split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .any(|part| part.len() >= MIN_REDACTED_PART && line.contains(part));

    if leaks {
        "[output withheld]".to_string()
    } else {
        line.clone()
    }
}

impl<I: HeraldIdentityProvisioner, R: HelmRunner> ClusterBootstrapper for HelmBootstrapper<I, R> {
    async fn bootstrap(&self, request: BootstrapRequest) -> Result<HeraldBinding, CoreError> {
        let minted = self.identities.mint(request.data_plane_id).await?;

        match self.install(&request, &minted).await {
            Ok(()) => {
                info!(data_plane = %request.data_plane_id.0, "installed the data plane chart");
                Ok(minted.binding.clone())
            }
            Err(error) => {
                if let Err(revoke) = self.identities.revoke(request.data_plane_id).await {
                    warn!(data_plane = %request.data_plane_id.0, error = %revoke, "could not revoke the identity after a failed install");
                }
                Err(error)
            }
        }
    }
}
