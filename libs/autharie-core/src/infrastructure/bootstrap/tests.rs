use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use autharie_domain::{
    CoreError,
    dataplane::{
        bootstrap::{BootstrapRequest, ClusterBootstrapper},
        credential::SecretString,
        herald_identity::{HeraldBinding, MintedHeraldIdentity},
        ports::HeraldIdentityProvisioner,
        value_objects::{DataPlaneId, Region},
    },
    organisation::OrganisationId,
};
use uuid::Uuid;

use super::{HelmBootstrapper, HelmConfig, HelmOutcome, HelmRunner, TokioHelmRunner};

const SECRET: &str = "s3cr3t-herald-value";
const KUBECONFIG: &str = "apiVersion: v1\nclusters: token-abcdef123456\n";

struct Identities {
    revoked: Mutex<Vec<DataPlaneId>>,
}

impl Identities {
    fn new() -> Self {
        Self {
            revoked: Mutex::new(Vec::new()),
        }
    }
}

impl HeraldIdentityProvisioner for &Identities {
    async fn mint(&self, dataplane: DataPlaneId) -> Result<MintedHeraldIdentity, CoreError> {
        Ok(MintedHeraldIdentity {
            binding: HeraldBinding {
                client_id: MintedHeraldIdentity::client_id_for(dataplane),
                subject: "subject-1".to_string(),
            },
            secret: SECRET.to_string(),
        })
    }

    async fn revoke(&self, dataplane: DataPlaneId) -> Result<(), CoreError> {
        self.revoked.lock().unwrap().push(dataplane);
        Ok(())
    }
}

struct Seen {
    args: Vec<String>,
    dir: PathBuf,
    values: String,
    kubeconfig: String,
    dir_mode: u32,
    values_mode: u32,
}

struct FakeRunner {
    outcome: HelmOutcome,
    seen: Mutex<Option<Seen>>,
}

impl FakeRunner {
    fn answering(code: i32, last_line: &str) -> Self {
        Self {
            outcome: HelmOutcome {
                code: Some(code),
                last_line: last_line.to_string(),
            },
            seen: Mutex::new(None),
        }
    }
}

impl HelmRunner for &FakeRunner {
    async fn run(&self, args: &[String], working_dir: &Path) -> Result<HelmOutcome, CoreError> {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        *self.seen.lock().unwrap() = Some(Seen {
            args: args.to_vec(),
            dir: working_dir.to_path_buf(),
            values: std::fs::read_to_string(working_dir.join("values.json")).unwrap(),
            kubeconfig: std::fs::read_to_string(working_dir.join("kubeconfig")).unwrap(),
            dir_mode: mode(working_dir),
            values_mode: mode(&working_dir.join("values.json")),
        });
        Ok(self.outcome.clone())
    }
}

fn request(id: u128) -> BootstrapRequest {
    BootstrapRequest {
        data_plane_id: DataPlaneId(Uuid::from_u128(id)),
        organisation_id: OrganisationId(Uuid::from_u128(9)),
        region: Region::new("fr-par"),
        kubeconfig: SecretString::new(KUBECONFIG),
    }
}

fn config() -> HelmConfig {
    HelmConfig::new("https://cp.example", "https://id.example/realms/autharie")
}

#[tokio::test]
async fn the_secret_is_in_the_values_file_and_not_on_the_command_line() {
    let identities = Identities::new();
    let runner = FakeRunner::answering(0, "");
    let bootstrapper = HelmBootstrapper::with_runner(&identities, &runner, config());

    bootstrapper.bootstrap(request(1)).await.unwrap();

    let seen = runner.seen.lock().unwrap();
    let seen = seen.as_ref().unwrap();
    assert!(seen.values.contains(SECRET));
    assert_eq!(seen.kubeconfig, KUBECONFIG);
    assert!(seen.args.iter().all(|a| !a.contains(SECRET)));
    assert!(seen.args.iter().all(|a| !a.contains("token-abcdef123456")));
    assert_eq!(seen.dir_mode, 0o700);
    assert_eq!(seen.values_mode, 0o600);
    for flag in ["--atomic", "--wait", "--create-namespace", "--kubeconfig"] {
        assert!(seen.args.iter().any(|a| a == flag), "{flag}");
    }
}

#[tokio::test]
async fn the_binding_returned_is_the_one_minted_and_the_directory_is_gone() {
    let identities = Identities::new();
    let runner = FakeRunner::answering(0, "");
    let bootstrapper = HelmBootstrapper::with_runner(&identities, &runner, config());

    let binding = bootstrapper.bootstrap(request(1)).await.unwrap();

    assert_eq!(binding.client_id, format!("herald-{}", Uuid::from_u128(1)));
    assert_eq!(binding.subject, "subject-1");
    assert!(identities.revoked.lock().unwrap().is_empty());
    let seen = runner.seen.lock().unwrap();
    assert!(!seen.as_ref().unwrap().dir.exists());
}

#[tokio::test]
async fn a_failed_install_revokes_the_identity_and_removes_the_directory() {
    let identities = Identities::new();
    let runner = FakeRunner::answering(1, "Error: release failed");
    let bootstrapper = HelmBootstrapper::with_runner(&identities, &runner, config());

    let error = bootstrapper.bootstrap(request(1)).await.unwrap_err();

    assert_eq!(
        *identities.revoked.lock().unwrap(),
        vec![DataPlaneId(Uuid::from_u128(1))]
    );
    let message = error.to_string();
    assert!(message.contains("exit 1"), "{message}");
    assert!(message.contains("release failed"), "{message}");
    assert!(!runner.seen.lock().unwrap().as_ref().unwrap().dir.exists());
}

#[tokio::test]
async fn an_error_never_echoes_the_secret_or_the_kubeconfig() {
    for echoed in [
        format!("Error: bad value {SECRET}"),
        "Error: token-abcdef123456 rejected".to_string(),
    ] {
        let identities = Identities::new();
        let runner = FakeRunner::answering(1, &echoed);
        let bootstrapper = HelmBootstrapper::with_runner(&identities, &runner, config());

        let message = bootstrapper
            .bootstrap(request(1))
            .await
            .unwrap_err()
            .to_string();

        assert!(!message.contains(SECRET), "{message}");
        assert!(!message.contains("token-abcdef123456"), "{message}");
        assert!(!message.contains(KUBECONFIG), "{message}");
    }
}

#[test]
fn the_version_is_pinned_only_when_configured() {
    let mut pinned = config();
    pinned.chart_version = Some("1.2.3".to_string());
    let with = super::bootstrapper::helm_args(&pinned, Path::new("k"), Path::new("v"));
    let without = super::bootstrapper::helm_args(&config(), Path::new("k"), Path::new("v"));

    assert!(with.windows(2).any(|w| w == ["--version", "1.2.3"]));
    assert!(!without.iter().any(|a| a == "--version"));
}

#[tokio::test]
async fn the_chart_renders_with_exactly_the_values_the_adapter_builds() {
    if std::process::Command::new("helm")
        .arg("version")
        .output()
        .is_err()
    {
        println!("skipped: helm is not on PATH");
        return;
    }

    let id = DataPlaneId(Uuid::from_u128(1));
    let minted = MintedHeraldIdentity {
        binding: HeraldBinding {
            client_id: MintedHeraldIdentity::client_id_for(id),
            subject: "subject-1".to_string(),
        },
        secret: SECRET.to_string(),
    };
    let mut config = config();
    config.image_registry = Some("registry.example".to_string());
    config.image_tag = Some("1.0.0".to_string());

    let workdir = super::workdir::Workdir::create().unwrap();
    let values_file = workdir
        .write_private(
            "values.json",
            super::bootstrapper::values(&config, id, &minted)
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let chart = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../charts/autharie-dataplane");
    let args = vec![
        "template".to_string(),
        "autharie-dataplane".to_string(),
        chart.to_string_lossy().into_owned(),
        "--values".to_string(),
        values_file.to_string_lossy().into_owned(),
    ];

    let outcome = TokioHelmRunner::new(PathBuf::from("helm"), Duration::from_secs(30))
        .run(&args, workdir.path())
        .await
        .unwrap();

    assert!(outcome.succeeded(), "{outcome:?}");
}
