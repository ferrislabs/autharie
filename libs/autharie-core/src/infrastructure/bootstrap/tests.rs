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
        ports::{HeraldBindingStore, HeraldIdentityProvisioner},
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

struct Bindings {
    bound: Mutex<Vec<(DataPlaneId, HeraldBinding)>>,
    unbound: Mutex<Vec<DataPlaneId>>,
}

impl Bindings {
    fn new() -> Self {
        Self {
            bound: Mutex::new(Vec::new()),
            unbound: Mutex::new(Vec::new()),
        }
    }
}

impl HeraldBindingStore for &Bindings {
    async fn bind(&self, dataplane: DataPlaneId, binding: &HeraldBinding) -> Result<(), CoreError> {
        self.bound
            .lock()
            .unwrap()
            .push((dataplane, binding.clone()));
        Ok(())
    }

    async fn unbind(&self, dataplane: DataPlaneId) -> Result<(), CoreError> {
        self.unbound.lock().unwrap().push(dataplane);
        Ok(())
    }
}

struct Seen {
    args: Vec<String>,
    dir: PathBuf,
    values: Option<String>,
    kubeconfig: String,
    dir_mode: u32,
    values_mode: Option<u32>,
}

struct FakeRunner {
    outcome: HelmOutcome,
    fail_at: Option<usize>,
    seen: Mutex<Vec<Seen>>,
}

impl FakeRunner {
    fn answering(code: i32, last_line: &str) -> Self {
        Self {
            outcome: HelmOutcome {
                code: Some(code),
                last_line: last_line.to_string(),
            },
            fail_at: None,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn failing_call(index: usize, last_line: &str) -> Self {
        Self {
            fail_at: Some(index),
            ..Self::answering(1, last_line)
        }
    }

    fn last(&self) -> Seen {
        self.seen.lock().unwrap().pop().unwrap()
    }
}

impl HelmRunner for &FakeRunner {
    async fn run(&self, args: &[String], working_dir: &Path) -> Result<HelmOutcome, CoreError> {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let values_path = args
            .windows(2)
            .find(|w| w[0] == "--values")
            .map(|w| PathBuf::from(&w[1]));
        let mut seen = self.seen.lock().unwrap();
        let index = seen.len();
        seen.push(Seen {
            args: args.to_vec(),
            dir: working_dir.to_path_buf(),
            values: values_path
                .as_ref()
                .map(|p| std::fs::read_to_string(p).unwrap()),
            kubeconfig: std::fs::read_to_string(working_dir.join("kubeconfig")).unwrap(),
            dir_mode: mode(working_dir),
            values_mode: values_path.as_ref().map(|p| mode(p)),
        });
        if self.fail_at.is_none_or(|at| at == index) {
            Ok(self.outcome.clone())
        } else {
            Ok(HelmOutcome {
                code: Some(0),
                last_line: String::new(),
            })
        }
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

fn chart_call(runner: &FakeRunner) -> Seen {
    runner.last()
}

#[tokio::test]
async fn the_secret_is_in_the_values_file_and_not_on_the_command_line() {
    let identities = Identities::new();
    let bindings = Bindings::new();
    let runner = FakeRunner::answering(0, "");
    let bootstrapper = HelmBootstrapper::with_runner(&identities, &bindings, &runner, config());

    bootstrapper.bootstrap(request(1)).await.unwrap();

    let seen = runner.seen.lock().unwrap();
    let values: serde_json::Value =
        serde_json::from_str(seen.last().unwrap().values.as_ref().unwrap()).unwrap();
    assert_eq!(values["controlPlane"]["auth"]["clientSecret"], SECRET);
    let password = values["rabbitmq"]["auth"]["password"].as_str().unwrap();
    assert!(password.len() >= 32);
    assert_ne!(password, "autharie");
    for call in seen.iter() {
        assert_eq!(call.kubeconfig, KUBECONFIG);
        assert!(call.args.iter().all(|a| !a.contains(SECRET)));
        assert!(call.args.iter().all(|a| !a.contains(password)));
        assert!(call.args.iter().all(|a| !a.contains("token-abcdef123456")));
        assert_eq!(call.dir_mode, 0o700);
        assert_eq!(call.values_mode, Some(0o600));
        for flag in ["--atomic", "--wait", "--create-namespace", "--kubeconfig"] {
            assert!(call.args.iter().any(|a| a == flag), "{flag}");
        }
    }
}

#[tokio::test]
async fn each_install_gets_its_own_rabbitmq_password() {
    let mut passwords = Vec::new();
    for _ in 0..2 {
        let identities = Identities::new();
        let bindings = Bindings::new();
        let runner = FakeRunner::answering(0, "");
        HelmBootstrapper::with_runner(&identities, &bindings, &runner, config())
            .bootstrap(request(1))
            .await
            .unwrap();
        let values: serde_json::Value =
            serde_json::from_str(chart_call(&runner).values.as_ref().unwrap()).unwrap();
        passwords.push(values["rabbitmq"]["auth"]["password"].to_string());
    }

    assert_ne!(passwords[0], passwords[1]);
}

#[tokio::test]
async fn prerequisites_are_installed_in_order_before_the_chart() {
    let identities = Identities::new();
    let bindings = Bindings::new();
    let runner = FakeRunner::answering(0, "");
    HelmBootstrapper::with_runner(&identities, &bindings, &runner, config())
        .bootstrap(request(1))
        .await
        .unwrap();

    let seen = runner.seen.lock().unwrap();
    let releases: Vec<&str> = seen.iter().map(|s| s.args[2].as_str()).collect();
    assert_eq!(releases, ["cnpg", "keda", "eg", "autharie-dataplane"]);
    for call in seen.iter() {
        assert_eq!(&call.args[..2], ["upgrade", "--install"]);
    }
    let prerequisites = &seen[..3];
    for call in prerequisites {
        assert!(call.args.windows(2).any(|w| w[0] == "--version"
            && !w[1].is_empty()
            && w[1].chars().next().unwrap().is_ascii_alphanumeric()));
    }
    assert!(seen[0].args.windows(2).any(|w| w[0] == "--repo"));
    assert!(seen[2].args.iter().all(|a| a != "--repo"));
    assert!(seen[2].args[3].starts_with("oci://"));
}

#[tokio::test]
async fn a_failing_prerequisite_stops_the_sequence_and_revokes() {
    let identities = Identities::new();
    let bindings = Bindings::new();
    let runner = FakeRunner::failing_call(1, "Error: keda not ready");
    let error = HelmBootstrapper::with_runner(&identities, &bindings, &runner, config())
        .bootstrap(request(1))
        .await
        .unwrap_err();

    let seen = runner.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(!seen[1].dir.exists());
    assert_eq!(
        *identities.revoked.lock().unwrap(),
        vec![DataPlaneId(Uuid::from_u128(1))]
    );
    let message = error.to_string();
    assert!(message.contains("prerequisite keda"), "{message}");
    assert!(message.contains("keda not ready"), "{message}");
}

#[tokio::test]
async fn the_binding_returned_is_the_one_minted_and_the_directory_is_gone() {
    let identities = Identities::new();
    let bindings = Bindings::new();
    let runner = FakeRunner::answering(0, "");
    let bootstrapper = HelmBootstrapper::with_runner(&identities, &bindings, &runner, config());

    let binding = bootstrapper.bootstrap(request(1)).await.unwrap();

    assert_eq!(binding.client_id, format!("herald-{}", Uuid::from_u128(1)));
    assert_eq!(binding.subject, "subject-1");
    assert!(identities.revoked.lock().unwrap().is_empty());
    assert!(!runner.last().dir.exists());
}

#[tokio::test]
async fn a_failed_chart_install_revokes_the_identity_and_removes_the_directory() {
    let identities = Identities::new();
    let bindings = Bindings::new();
    let runner = FakeRunner::failing_call(3, "Error: release failed");
    let bootstrapper = HelmBootstrapper::with_runner(&identities, &bindings, &runner, config());

    let error = bootstrapper.bootstrap(request(1)).await.unwrap_err();

    assert_eq!(
        *identities.revoked.lock().unwrap(),
        vec![DataPlaneId(Uuid::from_u128(1))]
    );
    let message = error.to_string();
    assert!(message.contains("exit 1"), "{message}");
    assert!(message.contains("data plane chart"), "{message}");
    assert!(message.contains("release failed"), "{message}");
    assert!(!runner.last().dir.exists());
}

#[tokio::test]
async fn an_error_never_echoes_a_secret_or_the_kubeconfig() {
    for failing in [0, 3] {
        for echoed in [
            format!("Error: bad value {SECRET}"),
            "Error: token-abcdef123456 rejected".to_string(),
        ] {
            let identities = Identities::new();
            let bindings = Bindings::new();
            let runner = FakeRunner::failing_call(failing, &echoed);
            let bootstrapper =
                HelmBootstrapper::with_runner(&identities, &bindings, &runner, config());

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
}

#[tokio::test]
async fn an_error_never_echoes_the_generated_password() {
    use std::sync::Arc;

    struct Echo(Mutex<Option<String>>);
    impl HelmRunner for Arc<Echo> {
        async fn run(&self, args: &[String], _: &Path) -> Result<HelmOutcome, CoreError> {
            let values = args.windows(2).find(|w| w[0] == "--values").unwrap();
            let content = std::fs::read_to_string(&values[1]).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
            match parsed["rabbitmq"]["auth"]["password"].as_str() {
                Some(password) => {
                    *self.0.lock().unwrap() = Some(password.to_string());
                    Ok(HelmOutcome {
                        code: Some(1),
                        last_line: format!("Error: amqp://u:{password}@host"),
                    })
                }
                None => Ok(HelmOutcome {
                    code: Some(0),
                    last_line: String::new(),
                }),
            }
        }
    }

    let identities = Identities::new();
    let bindings = Bindings::new();
    let echo = Arc::new(Echo(Mutex::new(None)));
    let message = HelmBootstrapper::with_runner(&identities, &bindings, echo.clone(), config())
        .bootstrap(request(1))
        .await
        .unwrap_err()
        .to_string();

    let password = echo.0.lock().unwrap().clone().unwrap();
    assert!(!message.contains(&password), "{message}");
    assert!(message.contains("withheld"), "{message}");
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
            super::bootstrapper::values(&config, id, &minted, "0123456789abcdef0123456789abcdef")
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

#[tokio::test]
async fn helm_keeps_its_cache_and_config_inside_the_working_directory() {
    let workdir = std::env::temp_dir().join(format!("helm-home-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&workdir).unwrap();
    let runner = TokioHelmRunner::new(PathBuf::from("sh"), Duration::from_secs(30));

    let mut seen = Vec::new();
    for variable in [
        "HOME",
        "HELM_CACHE_HOME",
        "HELM_CONFIG_HOME",
        "HELM_DATA_HOME",
    ] {
        let args = vec!["-c".to_string(), format!("echo \"${variable}\" >&2")];
        seen.push(runner.run(&args, &workdir).await.unwrap().last_line);
    }

    let root = workdir.to_string_lossy().into_owned();
    std::fs::remove_dir_all(&workdir).unwrap();
    assert_eq!(
        seen,
        vec![
            root.clone(),
            format!("{root}/helm-cache"),
            format!("{root}/helm-config"),
            format!("{root}/helm-data"),
        ]
    );
}

struct BindingWatcher<'a> {
    bindings: &'a Bindings,
    bound_when_helm_first_ran: Mutex<Option<usize>>,
}

impl HelmRunner for &BindingWatcher<'_> {
    async fn run(&self, _: &[String], _: &Path) -> Result<HelmOutcome, CoreError> {
        self.bound_when_helm_first_ran
            .lock()
            .unwrap()
            .get_or_insert(self.bindings.bound.lock().unwrap().len());
        Ok(HelmOutcome {
            code: Some(0),
            last_line: String::new(),
        })
    }
}

#[tokio::test]
async fn the_binding_is_recorded_before_the_first_chart_is_installed() {
    let identities = Identities::new();
    let bindings = Bindings::new();
    let runner = BindingWatcher {
        bindings: &bindings,
        bound_when_helm_first_ran: Mutex::new(None),
    };

    let binding = HelmBootstrapper::with_runner(&identities, &bindings, &runner, config())
        .bootstrap(request(1))
        .await
        .unwrap();

    assert_eq!(*runner.bound_when_helm_first_ran.lock().unwrap(), Some(1));
    assert_eq!(
        *bindings.bound.lock().unwrap(),
        vec![(DataPlaneId(Uuid::from_u128(1)), binding)]
    );
    assert!(bindings.unbound.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_install_takes_the_binding_back_with_the_identity() {
    let identities = Identities::new();
    let bindings = Bindings::new();
    let runner = FakeRunner::failing_call(0, "Error: boom");

    HelmBootstrapper::with_runner(&identities, &bindings, &runner, config())
        .bootstrap(request(1))
        .await
        .unwrap_err();

    let id = DataPlaneId(Uuid::from_u128(1));
    assert_eq!(*bindings.unbound.lock().unwrap(), vec![id]);
    assert_eq!(*identities.revoked.lock().unwrap(), vec![id]);
}
