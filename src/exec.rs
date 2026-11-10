//! Run the ABE helper scripts and turn their results into gRPC statuses.

use std::process::Command;
use tonic::Status;

/// Directory the Dockerfile installs the helper scripts into.
pub const SCRIPT_DIR: &str = "/opt/bin";

/// Run `SCRIPT_DIR/<script> <args...>`.
///
/// Returns trimmed stdout on success. The script's stderr is relayed into
/// our log either way so failures are debuggable from `kubectl logs`.
pub fn run(script: &str, args: &[String]) -> Result<String, Status> {
    let path = format!("{}/{}", SCRIPT_DIR, script);
    tracing::debug!("exec {} {:?}", path, args);

    let out = Command::new(&path)
        .args(args)
        .output()
        .map_err(|e| Status::internal(format!("failed to exec {}: {}", path, e)))?;

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
