//! Run the ABE helper scripts and turn their results into gRPC statuses.

use std::io::Write;
use std::process::{Command, ExitStatus, Stdio};
use tonic::Status;

/// Directory the Dockerfile installs the helper scripts into.
pub const SCRIPT_DIR: &str = "/opt/bin";

/// `abe-create-volume` exits with this (see `scripts/common.sh`) when
/// xdebbied answered 507: no ABE has room for the volume.
pub const EXIT_NO_SPACE: i32 = 28;

/// A helper script failed: which one, how it exited and what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub script: String,
    /// `None` when the script was killed by a signal.
    pub code: Option<i32>,
    pub stderr: String,
}

impl ScriptError {
    fn new(script: &str, status: ExitStatus, stderr: &str) -> Self {
        Self {
            script: script.to_string(),
            code: status.code(),
            stderr: stderr.trim().to_string(),
        }
    }

    /// The last line the script printed, without the `[abe][hh:mm:ss] `
    /// log prefix: what the scripts say right before they `die`.
    fn last_line(&self) -> &str {
        let line = self
            .stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        match line.strip_prefix("[abe][") {
            Some(rest) => rest.split_once("] ").map(|(_, m)| m).unwrap_or(line),
            None => line,
        }
    }
}

/// Which gRPC status a script failure becomes. Exit 28 from
/// `abe-create-volume` is "no ABE has room": `ResourceExhausted`, so the
/// external-provisioner puts a capacity problem on the PVC instead of a
/// generic internal error. Everything else is `Internal`.
impl From<ScriptError> for Status {
    fn from(e: ScriptError) -> Status {
        if e.script == "abe-create-volume" && e.code == Some(EXIT_NO_SPACE) {
            return Status::resource_exhausted(e.last_line().to_string());
        }
        let how = match e.code {
            Some(c) => format!("exit status: {}", c),
            None => "killed by a signal".to_string(),
        };
        Status::internal(format!("{} exited with {}: {}", e.script, how, e.stderr))
    }
}

/// Run `SCRIPT_DIR/<script> <args...>`.
///
/// Returns trimmed stdout on success. The script's stderr is relayed into
/// our log either way so failures are debuggable from `kubectl logs`.
pub fn run(script: &str, args: &[String]) -> Result<String, Status> {
    run_with_stdin(script, args, None)
}

/// Like [`run`], but feeds `stdin` to the script and sets
/// `ABE_LUKS_KEY_STDIN=1` so it knows to read it. Used for secrets, which
/// must never appear in argv or the environment.
pub fn run_with_stdin(
    script: &str,
    args: &[String],
    stdin: Option<&str>,
) -> Result<String, Status> {
    let path = format!("{}/{}", SCRIPT_DIR, script);
    tracing::debug!("exec {} {:?}", path, args);

    let mut cmd = Command::new(&path);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin.is_some() {
        cmd.stdin(Stdio::piped()).env("ABE_LUKS_KEY_STDIN", "1");
    } else {
        cmd.stdin(Stdio::null());
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| Status::internal(format!("failed to exec {}: {}", path, e)))?;

    if let Some(data) = stdin {
        if let Some(mut pipe) = child.stdin.take() {
            // Ignore EPIPE: a script that exits early will produce its own error.
            let _ = pipe.write_all(data.as_bytes());
        }
    }

    let out = child
        .wait_with_output()
        .map_err(|e| Status::internal(format!("waiting for {}: {}", path, e)))?;

    let stderr = String::from_utf8_lossy(&out.stderr);
    for line in stderr.lines().filter(|l| !l.trim().is_empty()) {
        tracing::info!(script, "{}", line);
    }

    if !out.status.success() {
        return Err(ScriptError::new(script, out.status, &stderr).into());
    }

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Flatten a string map into `KEY=VALUE` args for the scripts.
pub fn kv_args(map: &std::collections::HashMap<String, String>) -> Vec<String> {
    let mut v: Vec<String> = map.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    fn err(script: &str, code: Option<i32>, stderr: &str) -> ScriptError {
        ScriptError {
            script: script.to_string(),
            code,
            stderr: stderr.trim().to_string(),
        }
    }

    #[test]
    fn create_volume_exit_28_is_resource_exhausted_with_xdebbied_message() {
        let s: Status = err(
            "abe-create-volume",
            Some(EXIT_NO_SPACE),
            "[abe][07:27:00] creating 'pvc-x' (5368709120 bytes) via http://10.23.0.1:7443\n\
             [abe][07:27:01] no ABE has 5 GiB free (largest: 2 GiB on hv2)\n",
        )
        .into();
        assert_eq!(s.code(), Code::ResourceExhausted);
        assert_eq!(s.message(), "no ABE has 5 GiB free (largest: 2 GiB on hv2)");
    }

    #[test]
    fn create_volume_other_exits_stay_internal() {
        let s: Status = err(
            "abe-create-volume",
            Some(1),
            "[abe][07:27:00] xdebbied POST /volumes: 503: no healthy host",
        )
        .into();
        assert_eq!(s.code(), Code::Internal);
        assert!(s
            .message()
            .starts_with("abe-create-volume exited with exit status: 1: "));
        assert!(s.message().ends_with("503: no healthy host"));

        let killed: Status = err("abe-create-volume", None, "").into();
        assert_eq!(killed.code(), Code::Internal);
        assert!(killed.message().contains("killed by a signal"));
    }

    #[test]
    fn exit_28_from_other_scripts_is_not_special() {
        let s: Status = err("abe-delete-volume", Some(EXIT_NO_SPACE), "oops").into();
        assert_eq!(s.code(), Code::Internal);
        assert_eq!(
            s.message(),
            "abe-delete-volume exited with exit status: 28: oops"
        );
    }

    #[test]
    fn last_line_tolerates_missing_prefix() {
        assert_eq!(
            err("x", Some(28), "plain message\n\n").last_line(),
            "plain message"
        );
        assert_eq!(err("x", Some(28), "").last_line(), "");
        assert_eq!(err("x", Some(28), "[abe][1] odd").last_line(), "odd");
    }
}
