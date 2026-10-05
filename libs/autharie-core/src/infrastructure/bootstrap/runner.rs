use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use autharie_domain::CoreError;
use tokio::{io::AsyncReadExt, process::Command};

const MAX_LINE: usize = 200;
const GRACE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelmOutcome {
    pub code: Option<i32>,
    pub last_line: String,
}

impl HelmOutcome {
    pub fn succeeded(&self) -> bool {
        self.code == Some(0)
    }
}

pub trait HelmRunner: Send + Sync {
    fn run(
        &self,
        args: &[String],
        working_dir: &Path,
    ) -> impl Future<Output = Result<HelmOutcome, CoreError>> + Send;
}

pub struct TokioHelmRunner {
    binary: PathBuf,
    deadline: Duration,
}

impl TokioHelmRunner {
    pub fn new(binary: PathBuf, timeout: Duration) -> Self {
        Self {
            binary,
            deadline: timeout + GRACE,
        }
    }
}

pub(crate) fn sanitize(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| {
            line.chars()
                .filter(|c| !c.is_control())
                .take(MAX_LINE)
                .collect()
        })
        .unwrap_or_default()
}

impl HelmRunner for TokioHelmRunner {
    async fn run(&self, args: &[String], working_dir: &Path) -> Result<HelmOutcome, CoreError> {
        let mut child = Command::new(&self.binary)
            .args(args)
            .current_dir(working_dir)
            .env("HOME", working_dir)
            .env("HELM_CACHE_HOME", working_dir.join("helm-cache"))
            .env("HELM_CONFIG_HOME", working_dir.join("helm-config"))
            .env("HELM_DATA_HOME", working_dir.join("helm-data"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| CoreError::InternalError(format!("could not start helm: {}", e.kind())))?;

        let mut stderr = child.stderr.take();
        let wait = async {
            let mut collected = Vec::new();
            if let Some(pipe) = stderr.as_mut() {
                let _ = pipe.take(64 * 1024).read_to_end(&mut collected).await;
            }
            let status = child.wait().await;
            (status, collected)
        };

        let (status, collected) = tokio::time::timeout(self.deadline, wait)
            .await
            .map_err(|_| CoreError::InternalError("helm did not finish in time".to_string()))?;

        let status = status.map_err(|e| {
            CoreError::InternalError(format!("could not wait for helm: {}", e.kind()))
        })?;

        Ok(HelmOutcome {
            code: status.code(),
            last_line: sanitize(&String::from_utf8_lossy(&collected)),
        })
    }
}
