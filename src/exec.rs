//! Run the ABE helper scripts and turn their results into gRPC statuses.

use std::io::Write;
use std::process::{Command, Stdio};
use tonic::Status;

/// Directory the Dockerfile installs the helper scripts into.
pub const SCRIPT_DIR: &str = "/opt/bin";

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
pub fn run_with_stdin(script: &str, args: &[String], stdin: Option<&str>) -> Result<String, Status> {
    let path = format!("{}/{}", SCRIPT_DIR, script);
    tracing::debug!("exec {} {:?}", path, args);

    let mut cmd = Command::new(&path);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
        return Err(Status::internal(format!(
            "{} exited with {}: {}",
            script,
            out.status,
            stderr.trim()
        )));
    }

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Flatten a string map into `KEY=VALUE` args for the scripts.
pub fn kv_args(map: &std::collections::HashMap<String, String>) -> Vec<String> {
    let mut v: Vec<String> = map.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
    v.sort();
    v
}
