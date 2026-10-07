use autharie_domain::{
    CoreError,
    dataplane::{
        bootstrap::{BootstrapRequest, ClusterBootstrapper},
        herald_identity::{HeraldBinding, MintedHeraldIdentity},
        ports::{HeraldBindingStore, HeraldIdentityProvisioner},
        value_objects::DataPlaneId,
    },
};
use std::{path::Path, time::Duration};

use serde_json::{Value, json};
use tracing::{info, warn};
use uuid::Uuid;
use zeroize::Zeroizing;

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
const PASSWORD_PART: usize = 32;

pub struct HelmBootstrapper<I, B, R = TokioHelmRunner> {
    identities: I,
    bindings: B,
    runner: R,
    config: HelmConfig,
}

impl<I: HeraldIdentityProvisioner, B: HeraldBindingStore> HelmBootstrapper<I, B> {
    pub fn new(identities: I, bindings: B, config: HelmConfig) -> Self {
        let runner = TokioHelmRunner::new(config.helm_binary.clone(), config.timeout);
        Self {
            identities,
            bindings,
            runner,
            config,
        }
    }
}

impl<I: HeraldIdentityProvisioner, B: HeraldBindingStore, R: HelmRunner> HelmBootstrapper<I, B, R> {
    pub fn with_runner(identities: I, bindings: B, runner: R, config: HelmConfig) -> Self {
        Self {
            identities,
            bindings,
            runner,
            config,
        }
    }

    async fn install(
        &self,
        request: &BootstrapRequest,
        minted: &MintedHeraldIdentity,
    ) -> Result<(), CoreError> {
        let password = generate_password();
        let workdir = Workdir::create()?;
        let kubeconfig =
            workdir.write_private(KUBECONFIG_FILE, request.kubeconfig.expose().as_bytes())?;
        let secrets = [
            minted.secret.as_str(),
            request.kubeconfig.expose(),
            password.as_str(),
        ];

        for (index, prerequisite) in self.config.prerequisites.iter().enumerate() {
            let values = workdir.write_private(
                &format!("prerequisite-{index}.json"),
                prerequisite.values.to_string().as_bytes(),
            )?;
            let invocation = Invocation {
                release: &prerequisite.release,
                chart: &prerequisite.chart,
                repo: prerequisite.repo.as_deref(),
                version: Some(&prerequisite.version),
                namespace: &prerequisite.namespace,
            };
            let args = upgrade_args(&invocation, self.config.timeout, &kubeconfig, &values);
            self.run_step(
                &args,
                &workdir,
                &secrets,
                &format!("the prerequisite {}", prerequisite.release),
            )
            .await?;
        }

        let values = workdir.write_private(
            VALUES_FILE,
            values(&self.config, request.data_plane_id, minted, &password)
                .to_string()
                .as_bytes(),
        )?;
        let args = helm_args(&self.config, &kubeconfig, &values);
        self.run_step(&args, &workdir, &secrets, "the data plane chart")
            .await
    }

    async fn run_step(
        &self,
        args: &[String],
        workdir: &Workdir,
        secrets: &[&str],
        what: &str,
    ) -> Result<(), CoreError> {
        let outcome = self.runner.run(args, workdir.path()).await?;

        if outcome.succeeded() {
            return Ok(());
        }

        Err(CoreError::InternalError(format!(
            "installing {what} failed (exit {}): {}",
            outcome
                .code
                .map_or_else(|| "signal".to_string(), |c| c.to_string()),
            redact(&outcome, secrets),
        )))
    }
}

struct Invocation<'a> {
    release: &'a str,
    chart: &'a str,
    repo: Option<&'a str>,
    version: Option<&'a str>,
    namespace: &'a str,
}

fn generate_password() -> Zeroizing<String> {
    let mut password = Zeroizing::new(String::with_capacity(2 * PASSWORD_PART));
    for _ in 0..2 {
        let mut buffer = Zeroizing::new([0u8; PASSWORD_PART]);
        password.push_str(Uuid::new_v4().simple().encode_lower(&mut *buffer));
    }
    password
}

pub(crate) fn values(
    config: &HelmConfig,
    id: DataPlaneId,
    minted: &MintedHeraldIdentity,
    rabbitmq_password: &str,
) -> Value {
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
        "rabbitmq": { "auth": { "password": rabbitmq_password } },
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
    let invocation = Invocation {
        release: RELEASE,
        chart: &config.chart,
        repo: None,
        version: config.chart_version.as_deref(),
        namespace: NAMESPACE,
    };
    upgrade_args(&invocation, config.timeout, kubeconfig, values)
}

fn upgrade_args(
    invocation: &Invocation<'_>,
    timeout: Duration,
    kubeconfig: &Path,
    values: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "upgrade",
        "--install",
        invocation.release,
        invocation.chart,
        "--namespace",
        invocation.namespace,
        "--create-namespace",
        "--wait",
        "--atomic",
        "--timeout",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    args.push(format!("{}s", timeout.as_secs()));
    args.push("--kubeconfig".to_string());
    args.push(kubeconfig.to_string_lossy().into_owned());
    args.push("--values".to_string());
    args.push(values.to_string_lossy().into_owned());

    if let Some(repo) = invocation.repo {
        args.push("--repo".to_string());
        args.push(repo.to_string());
    }
    if let Some(version) = invocation.version {
        args.push("--version".to_string());
        args.push(version.to_string());
    }

    args
}

fn redact(outcome: &HelmOutcome, secrets: &[&str]) -> String {
    let line = &outcome.last_line;
    let leaks = secrets.iter().any(|secret| {
        (!secret.is_empty() && line.contains(secret))
            || secret
                .split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .any(|part| part.len() >= MIN_REDACTED_PART && line.contains(part))
    });

    if leaks {
        "[output withheld]".to_string()
    } else {
        line.clone()
    }
}

impl<I: HeraldIdentityProvisioner, B: HeraldBindingStore, R: HelmRunner> ClusterBootstrapper
    for HelmBootstrapper<I, B, R>
{
    async fn bootstrap(&self, request: BootstrapRequest) -> Result<HeraldBinding, CoreError> {
        let minted = self.identities.mint(request.data_plane_id).await?;

        if let Err(error) = self
            .bindings
            .bind(request.data_plane_id, &minted.binding)
            .await
        {
            self.abandon(&request).await;
            return Err(error);
        }

        match self.install(&request, &minted).await {
            Ok(()) => {
                info!(data_plane = %request.data_plane_id.0, "installed the data plane chart");
                Ok(minted.binding.clone())
            }
            Err(error) => {
                self.abandon(&request).await;
                Err(error)
            }
        }
    }
}

impl<I: HeraldIdentityProvisioner, B: HeraldBindingStore, R: HelmRunner> HelmBootstrapper<I, B, R> {
    async fn abandon(&self, request: &BootstrapRequest) {
        let id = request.data_plane_id;
        if let Err(error) = self.bindings.unbind(id).await {
            warn!(data_plane = %id.0, %error, "could not take the binding back after a failed install");
        }
        if let Err(error) = self.identities.revoke(id).await {
            warn!(data_plane = %id.0, %error, "could not revoke the identity after a failed install");
        }
    }
}
