//! The daily dump of a shared database, as a CronJob in the tenant's namespace.
//!
//! A free deployment has no cluster of its own, so there is no CloudNativePG
//! base backup to lean on. What it has instead is `pg_dump` of its one
//! database, written to the same object store the paid deployments archive to.
//! Like `archive.rs` this is a pure function from a few facts to a resource:
//! what it builds is worth testing, and none of it is worth a cluster to test.
//!
//! The CronJob lives in the tenant namespace and is owned by the instance, so
//! it goes with them. The dumps in the object store are deliberately not
//! touched on deletion: they are the recovery, and a recovery that disappears
//! with the thing it recovers is not one.

use autharie_crds::v1alpha::identity_instance::BackupConfig;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::OperatorError;
use crate::infrastructure::archive::{ACCESS_KEY_ID, ACCESS_SECRET_KEY, split_destination};

/// How many dumps stay under a deployment's prefix. A week of dailies: enough
/// to notice a problem on Monday that started on Wednesday, and small enough
/// that a free tenant's storage stays a rounding error.
pub const KEPT_DUMPS: usize = 7;

/// Pinned, never `latest`: a dump that silently changes client version is a
/// restore that changes behaviour a year later.
pub const UPLOAD_IMAGE: &str = "amazon/aws-cli:2.27.50";

const INSTANCE_PREFIX: &str = "deployment-";
const DUMP_FILE: &str = "/dump/database.dump";
const DEFAULT_ENCRYPTION: &str = "AES256";

/// uid and gid of `postgres` in the CloudNativePG image. Both containers run as
/// it so the file one writes is the file the other can read.
const RUN_AS: i64 = 26;

/// Dumps start between 01:00 and 05:00 UTC, which is night for the regions this
/// platform runs in and keeps clear of the 02:30 base backups of paid clusters.
const WINDOW_START_HOUR: u128 = 1;
const WINDOW_MINUTES: u128 = 4 * 60;

/// Everything the CronJob needs and nothing it should work out itself.
pub struct DumpTarget<'a> {
    pub instance_name: &'a str,
    pub namespace: &'a str,
    pub database_secret: &'a str,
    pub backup: &'a BackupConfig,
    pub region: &'a str,
    /// The image the shared cluster is running, as CloudNativePG reports it.
    ///
    /// `pg_dump` refuses a server newer than itself, and the cluster's major
    /// moves with the operator's default image, so any image written down
    /// here would one day be the wrong one and fail every night without a
    /// word. It is asked of the cluster instead, and passed in so this stays a
    /// pure function.
    pub postgres_image: &'a str,
}

/// The image a CloudNativePG `Cluster` says it is running, if it says.
///
/// `None` until the cluster has been reconciled once. The caller waits rather
/// than guessing: a guess that is one major behind is a dump job that exists
/// and never succeeds.
pub fn cluster_image(cluster: &Value) -> Option<&str> {
    cluster
        .get("status")
        .and_then(|status| status.get("image"))
        .and_then(Value::as_str)
        .filter(|image| !image.is_empty())
}

pub fn cronjob_name(instance_name: &str) -> String {
    format!("{instance_name}-dump")
}

pub fn deployment_id(instance_name: &str) -> Result<Uuid, OperatorError> {
    instance_name
        .strip_prefix(INSTANCE_PREFIX)
        .and_then(|id| Uuid::parse_str(id).ok())
        .ok_or_else(|| OperatorError::Configuration {
            message: format!(
                "`{instance_name}` is not named after a deployment id, so its dumps have no prefix"
            ),
        })
}

/// A fixed time per deployment, taken from the id.
///
/// Spread over a window so that a data plane full of free tenants does not
/// open that many connections to the shared cluster in the same minute. From
/// the id rather than from a random draw, so a reconcile never moves the
/// schedule: a time that changed on every apply would be a dump that sometimes
/// ran twice a day and sometimes not at all.
pub fn schedule(deployment: Uuid) -> String {
    let slot = deployment.as_u128() % WINDOW_MINUTES;
    let hour = WINDOW_START_HOUR + slot / 60;
    let minute = slot % 60;
    format!("{minute} {hour} * * *")
}

/// Where a deployment's dumps are written, relative to the bucket.
pub fn prefix(deployment: Uuid) -> String {
    format!("dumps/{deployment}/")
}

/// The script of the upload container.
///
/// Everything variable comes in through the environment, so the text is the
/// same for every tenant and there is no value in it to quote.
///
/// The names sort the same as the instants they stand for, so keeping the
/// newest seven is a sort on the key and nothing cleverer. `tr` splits the
/// list because the CLI prints a text list tab separated on one line. The list
/// goes through a file first so a failed listing stops the job instead of
/// looking like nothing to delete.
fn upload_script() -> String {
    format!(
        r#"set -eu
printf '[default]\ns3 =\n  addressing_style = path\n' > /tmp/aws-config
KEY="${{PREFIX}}$(date -u +%Y%m%dT%H%M%SZ).dump"
aws s3 cp {DUMP_FILE} "s3://${{BUCKET}}/${{KEY}}" --sse "${{SSE}}" ${{ENDPOINT:+--endpoint-url "${{ENDPOINT}}"}}
aws s3api list-objects-v2 --bucket "${{BUCKET}}" --prefix "${{PREFIX}}" \
  --query 'sort_by(Contents,&Key)[:-{KEPT_DUMPS}].Key' --output text \
  ${{ENDPOINT:+--endpoint-url "${{ENDPOINT}}"}} > /tmp/old-dumps
tr '\t' '\n' < /tmp/old-dumps | while read -r OLD; do
  if [ -n "${{OLD}}" ] && [ "${{OLD}}" != "None" ]; then
    aws s3api delete-object --bucket "${{BUCKET}}" --key "${{OLD}}" ${{ENDPOINT:+--endpoint-url "${{ENDPOINT}}"}}
  fi
done
"#
    )
}

fn secret_env(name: &str, secret: &str, key: &str) -> Value {
    json!({
        "name": name,
        "valueFrom": { "secretKeyRef": { "name": secret, "key": key } },
    })
}

fn security_context() -> Value {
    json!({
        "allowPrivilegeEscalation": false,
        "readOnlyRootFilesystem": true,
        "capabilities": { "drop": ["ALL"] },
    })
}

pub fn build_dump_cronjob(
    target: &DumpTarget<'_>,
    owner_reference: Option<Value>,
) -> Result<Value, OperatorError> {
    let deployment = deployment_id(target.instance_name)?;
    let (bucket, _) = split_destination(&target.backup.destination_path).ok_or_else(|| {
        OperatorError::Configuration {
            message: format!(
                "'{}' is not an S3 destination, so dumps have nowhere to go",
                target.backup.destination_path
            ),
        }
    })?;
    let name = cronjob_name(target.instance_name);
    let sse = target
        .backup
        .encryption
        .as_deref()
        .unwrap_or(DEFAULT_ENCRYPTION);
    let credentials = &target.backup.credentials_secret;
    let labels = json!({
        "app.kubernetes.io/name": "database-dump",
        "app.kubernetes.io/managed-by": "autharie",
        "app.kubernetes.io/instance": target.instance_name,
    });

    let dump = json!({
        "name": "pg-dump",
        "image": target.postgres_image,
        "command": ["pg_dump", "-Fc", "--no-owner", "--no-acl", "--file", DUMP_FILE],
        "env": [
            secret_env("PGHOST", target.database_secret, "host"),
            secret_env("PGPORT", target.database_secret, "port"),
            secret_env("PGDATABASE", target.database_secret, "database"),
            secret_env("PGUSER", target.database_secret, "user"),
            secret_env("PGPASSWORD", target.database_secret, "password"),
        ],
        "volumeMounts": [{ "name": "dump", "mountPath": "/dump" }],
        "resources": {
            "requests": { "cpu": "50m", "memory": "64Mi" },
            "limits": { "memory": "256Mi" },
        },
        "securityContext": security_context(),
    });

    let mut upload_env = vec![
        secret_env("AWS_ACCESS_KEY_ID", credentials, ACCESS_KEY_ID),
        secret_env("AWS_SECRET_ACCESS_KEY", credentials, ACCESS_SECRET_KEY),
        json!({ "name": "AWS_DEFAULT_REGION", "value": target.region }),
        json!({ "name": "AWS_CONFIG_FILE", "value": "/tmp/aws-config" }),
        json!({ "name": "AWS_EC2_METADATA_DISABLED", "value": "true" }),
        json!({ "name": "AWS_REQUEST_CHECKSUM_CALCULATION", "value": "when_required" }),
        json!({ "name": "AWS_RESPONSE_CHECKSUM_VALIDATION", "value": "when_required" }),
        json!({ "name": "HOME", "value": "/tmp" }),
        json!({ "name": "BUCKET", "value": bucket }),
        json!({ "name": "PREFIX", "value": prefix(deployment) }),
        json!({ "name": "SSE", "value": sse }),
    ];
    if let Some(endpoint) = &target.backup.endpoint_url {
        upload_env.push(json!({ "name": "ENDPOINT", "value": endpoint }));
    }

    let upload = json!({
        "name": "upload",
        "image": UPLOAD_IMAGE,
        "command": ["/bin/sh", "-c", upload_script()],
        "env": upload_env,
        "volumeMounts": [
            { "name": "dump", "mountPath": "/dump", "readOnly": true },
            { "name": "tmp", "mountPath": "/tmp" },
        ],
        "resources": {
            "requests": { "cpu": "50m", "memory": "64Mi" },
            "limits": { "memory": "256Mi" },
        },
        "securityContext": security_context(),
    });

    Ok(json!({
        "apiVersion": "batch/v1",
        "kind": "CronJob",
        "metadata": {
            "name": name,
            "namespace": target.namespace,
            "labels": labels,
            "ownerReferences": owner_reference.map(|owner| vec![owner]),
        },
        "spec": {
            "schedule": schedule(deployment),
            "timeZone": "Etc/UTC",
            // A dump still running at the next tick is the one to wait for,
            // not to race: two writers to one prefix make the retention
            // count lie.
            "concurrencyPolicy": "Forbid",
            "startingDeadlineSeconds": 3600,
            "successfulJobsHistoryLimit": 1,
            "failedJobsHistoryLimit": 3,
            "jobTemplate": {
                "spec": {
                    "backoffLimit": 2,
                    "activeDeadlineSeconds": 3600,
                    "template": {
                        "metadata": { "labels": labels },
                        "spec": {
                            "restartPolicy": "Never",
                            "automountServiceAccountToken": false,
                            "securityContext": {
                                "runAsNonRoot": true,
                                "runAsUser": RUN_AS,
                                "runAsGroup": RUN_AS,
                                "fsGroup": RUN_AS,
                                "seccompProfile": { "type": "RuntimeDefault" },
                            },
                            "initContainers": [dump],
                            "containers": [upload],
                            "volumes": [
                                {
                                    "name": "dump",
                                    "emptyDir": { "sizeLimit": "2Gi" },
                                },
                                { "name": "tmp", "emptyDir": { "sizeLimit": "64Mi" } },
                            ],
                        },
                    },
                },
            },
        },
    }))
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::batch::v1::CronJob;

    use super::*;

    const INSTANCE: &str = "deployment-b6a1c2d3-e4f5-4a6b-8c9d-0e1f2a3b4c5d";
    const IMAGE: &str = "ghcr.io/cloudnative-pg/postgresql:18.3-system-trixie";
    const ID: &str = "b6a1c2d3-e4f5-4a6b-8c9d-0e1f2a3b4c5d";

    fn backup() -> BackupConfig {
        BackupConfig {
            destination_path: "s3://autharie-backups/org-1/deployment-1".to_string(),
            endpoint_url: Some("http://rustfs:9000".to_string()),
            credentials_secret: "autharie-object-store".to_string(),
            encryption: Some("AES256".to_string()),
        }
    }

    fn build(backup: &BackupConfig) -> Value {
        build_dump_cronjob(
            &DumpTarget {
                instance_name: INSTANCE,
                namespace: "tenant-a",
                database_secret: "deployment-db",
                backup,
                region: "us-east-1",
                postgres_image: IMAGE,
            },
            None,
        )
        .expect("a cronjob")
    }

    fn env_of<'a>(container: &'a Value, name: &str) -> &'a Value {
        container["env"]
            .as_array()
            .and_then(|vars| vars.iter().find(|var| var["name"] == name))
            .unwrap_or_else(|| panic!("no env var {name}"))
    }

    fn pod(job: &Value) -> &Value {
        &job["spec"]["jobTemplate"]["spec"]["template"]["spec"]
    }

    #[test]
    fn the_schedule_is_the_same_every_time_for_one_deployment() {
        let id = Uuid::parse_str(ID).unwrap();

        assert_eq!(schedule(id), schedule(id));
        assert_eq!(build(&backup())["spec"]["schedule"], schedule(id));
    }

    #[test]
    fn the_schedule_stays_inside_the_night_window() {
        for n in 0..5000u128 {
            let schedule = schedule(Uuid::from_u128(n.wrapping_mul(0x9E37_79B9_7F4A_7C15)));
            let mut fields = schedule.split(' ');
            let minute: u32 = fields.next().unwrap().parse().unwrap();
            let hour: u32 = fields.next().unwrap().parse().unwrap();

            assert!(minute < 60);
            assert!((1..5).contains(&hour), "{schedule}");
            assert_eq!(fields.collect::<Vec<_>>(), ["*", "*", "*"]);
        }
    }

    #[test]
    fn deployments_do_not_all_dump_in_the_same_minute() {
        let schedules: std::collections::BTreeSet<String> =
            (0..64u128).map(|n| schedule(Uuid::from_u128(n))).collect();

        assert_eq!(schedules.len(), 64);
    }

    #[test]
    fn the_job_is_a_daily_cronjob_that_never_overlaps() {
        let job = build(&backup());

        assert_eq!(job["apiVersion"], "batch/v1");
        assert_eq!(job["kind"], "CronJob");
        assert_eq!(job["metadata"]["namespace"], "tenant-a");
        assert_eq!(job["spec"]["concurrencyPolicy"], "Forbid");
        assert_eq!(job["spec"]["timeZone"], "Etc/UTC");
        assert_eq!(job["spec"]["successfulJobsHistoryLimit"], 1);
        assert_eq!(job["spec"]["failedJobsHistoryLimit"], 3);
    }

    #[test]
    fn the_name_fits_what_a_cronjob_may_be_called() {
        assert!(cronjob_name(INSTANCE).len() <= 52);
    }

    #[test]
    fn it_deserialises_as_a_real_cronjob() {
        let job: CronJob = serde_json::from_value(build(&backup())).expect("a CronJob");

        assert_eq!(
            job.spec.expect("a spec").schedule,
            schedule(Uuid::parse_str(ID).unwrap())
        );
    }

    #[test]
    fn the_database_is_dumped_in_custom_format_by_an_init_container() {
        let job = build(&backup());
        let init = &pod(&job)["initContainers"][0];

        assert_eq!(init["image"], IMAGE);
        assert_eq!(init["command"][0], "pg_dump");
        assert_eq!(init["command"][1], "-Fc");
    }

    #[test]
    fn the_connection_comes_from_the_tenant_secret_and_nothing_is_inline() {
        let job = build(&backup());
        let init = &pod(&job)["initContainers"][0];

        for (var, key) in [
            ("PGHOST", "host"),
            ("PGPORT", "port"),
            ("PGDATABASE", "database"),
            ("PGUSER", "user"),
            ("PGPASSWORD", "password"),
        ] {
            let reference = &env_of(init, var)["valueFrom"]["secretKeyRef"];
            assert_eq!(reference["name"], "deployment-db");
            assert_eq!(reference["key"], key);
            assert!(env_of(init, var).get("value").is_none());
        }
    }

    #[test]
    fn the_store_credentials_come_from_the_secret_the_operator_writes() {
        let job = build(&backup());
        let upload = &pod(&job)["containers"][0];

        let access = &env_of(upload, "AWS_ACCESS_KEY_ID")["valueFrom"]["secretKeyRef"];
        let secret = &env_of(upload, "AWS_SECRET_ACCESS_KEY")["valueFrom"]["secretKeyRef"];
        assert_eq!(access["name"], "autharie-object-store");
        assert_eq!(access["key"], ACCESS_KEY_ID);
        assert_eq!(secret["name"], "autharie-object-store");
        assert_eq!(secret["key"], ACCESS_SECRET_KEY);
    }

    #[test]
    fn no_secret_value_appears_anywhere_in_the_rendered_job() {
        let rendered = build(&backup()).to_string();

        assert!(!rendered.contains("autharieautharie"));
        assert!(!rendered.contains("postgres://"));
    }

    #[test]
    fn dumps_go_under_the_deployment_inside_the_bucket_not_the_archive_prefix() {
        let job = build(&backup());
        let upload = &pod(&job)["containers"][0];

        assert_eq!(env_of(upload, "BUCKET")["value"], "autharie-backups");
        assert_eq!(env_of(upload, "PREFIX")["value"], format!("dumps/{ID}/"));
    }

    #[test]
    fn the_upload_asks_for_server_side_encryption() {
        let job = build(&backup());
        let upload = &pod(&job)["containers"][0];
        let script = upload["command"][2].as_str().unwrap();

        assert_eq!(env_of(upload, "SSE")["value"], "AES256");
        assert!(script.contains("--sse \"${SSE}\""));
    }

    #[test]
    fn an_instance_that_names_no_encryption_is_still_encrypted() {
        let mut config = backup();
        config.encryption = None;
        let job = build(&config);

        assert_eq!(
            env_of(&pod(&job)["containers"][0], "SSE")["value"],
            "AES256"
        );
    }

    #[test]
    fn a_kms_request_is_passed_through() {
        let mut config = backup();
        config.encryption = Some("aws:kms".to_string());
        let job = build(&config);

        assert_eq!(
            env_of(&pod(&job)["containers"][0], "SSE")["value"],
            "aws:kms"
        );
    }

    #[test]
    fn the_seven_newest_dumps_are_kept() {
        let job = build(&backup());
        let script = pod(&job)["containers"][0]["command"][2].as_str().unwrap();

        assert_eq!(KEPT_DUMPS, 7);
        assert!(script.contains("sort_by(Contents,&Key)[:-7].Key"));
        assert!(script.contains("delete-object"));
    }

    #[test]
    fn the_store_is_addressed_by_path_like_every_other_client_here() {
        let job = build(&backup());
        let upload = &pod(&job)["containers"][0];

        assert!(
            upload["command"][2]
                .as_str()
                .unwrap()
                .contains("addressing_style = path")
        );
        assert_eq!(env_of(upload, "ENDPOINT")["value"], "http://rustfs:9000");
    }

    #[test]
    fn without_an_endpoint_the_variable_is_absent_so_aws_is_used() {
        let mut config = backup();
        config.endpoint_url = None;
        let job = build(&config);
        let vars = pod(&job)["containers"][0]["env"]
            .as_array()
            .unwrap()
            .clone();

        assert!(vars.iter().all(|var| var["name"] != "ENDPOINT"));
    }

    #[test]
    fn the_pod_is_unprivileged_and_has_no_api_token() {
        let job = build(&backup());
        let pod = pod(&job);

        assert_eq!(pod["automountServiceAccountToken"], false);
        assert_eq!(pod["securityContext"]["runAsNonRoot"], true);
        for container in [&pod["initContainers"][0], &pod["containers"][0]] {
            assert_eq!(
                container["securityContext"]["allowPrivilegeEscalation"],
                false
            );
            assert!(container["resources"]["limits"]["memory"].is_string());
        }
    }

    #[test]
    fn the_dump_runs_the_image_it_is_given() {
        let job = build_dump_cronjob(
            &DumpTarget {
                instance_name: INSTANCE,
                namespace: "tenant-a",
                database_secret: "deployment-db",
                backup: &backup(),
                region: "us-east-1",
                postgres_image: "registry.example/postgresql:17.4",
            },
            None,
        )
        .unwrap();

        assert_eq!(
            pod(&job)["initContainers"][0]["image"],
            "registry.example/postgresql:17.4"
        );
        assert_eq!(pod(&build(&backup()))["initContainers"][0]["image"], IMAGE);
    }

    #[test]
    fn the_cluster_image_is_read_from_its_status_and_only_from_there() {
        let ready = json!({
            "spec": { "imageName": "spec-only" },
            "status": { "image": IMAGE },
        });

        assert_eq!(cluster_image(&ready), Some(IMAGE));
        assert_eq!(
            cluster_image(&json!({ "spec": { "imageName": "x" } })),
            None
        );
        assert_eq!(cluster_image(&json!({ "status": {} })), None);
        assert_eq!(cluster_image(&json!({ "status": { "image": "" } })), None);
    }

    #[test]
    fn the_images_are_pinned() {
        assert!(UPLOAD_IMAGE.contains(':'));
        assert!(!UPLOAD_IMAGE.ends_with(":latest"));
    }

    #[test]
    fn a_destination_that_is_not_s3_is_refused() {
        let mut config = backup();
        config.destination_path = "/var/backups".to_string();

        let result = build_dump_cronjob(
            &DumpTarget {
                instance_name: INSTANCE,
                namespace: "tenant-a",
                database_secret: "deployment-db",
                backup: &config,
                region: "us-east-1",
                postgres_image: IMAGE,
            },
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn an_instance_not_named_after_a_deployment_is_refused() {
        assert!(deployment_id("acme").is_err());
        assert_eq!(
            deployment_id(INSTANCE).unwrap(),
            Uuid::parse_str(ID).unwrap()
        );
    }
}
