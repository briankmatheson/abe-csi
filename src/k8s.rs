//! Just enough of the Kubernetes API to annotate a PersistentVolume.
//!
//! The controller runs in-cluster with the provisioner's ServiceAccount,
//! which already has `get`/`patch` on persistentvolumes. We talk to the API
//! server directly rather than pulling in a full client: one GET and one
//! merge-patch is all we need.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

const SA_DIR: &str = "/var/run/secrets/kubernetes.io/serviceaccount";

pub struct Client {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Client {
    /// Build a client from the in-cluster ServiceAccount mount.
    pub fn in_cluster() -> Result<Self> {
        let host = std::env::var("KUBERNETES_SERVICE_HOST")
            .context("KUBERNETES_SERVICE_HOST not set (not running in-cluster?)")?;
        let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
        let token = std::fs::read_to_string(format!("{SA_DIR}/token"))
            .context("reading ServiceAccount token")?
            .trim()
            .to_string();
        let ca = std::fs::read(format!("{SA_DIR}/ca.crt")).context("reading cluster CA")?;
        let http = reqwest::Client::builder()
            .add_root_certificate(reqwest::Certificate::from_pem(&ca)?)
            .timeout(Duration::from_secs(15))
            .build()?;
        // IPv6 service hosts need brackets.
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        Ok(Self {
            base: format!("https://{host}:{port}"),
            token,
            http,
        })
    }

    fn pv_url(&self, name: &str) -> String {
        format!("{}/api/v1/persistentvolumes/{}", self.base, name)
    }

    /// The PV as JSON, or `None` if it does not exist (yet).
    pub async fn get_pv(&self, name: &str) -> Result<Option<Value>> {
        let resp = self
            .http
            .get(self.pv_url(name))
            .bearer_auth(&self.token)
            .send()
            .await?;
        match resp.status() {
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            s if s.is_success() => Ok(Some(resp.json().await?)),
            s => Err(anyhow!(
                "GET pv/{name}: {s} {}",
                resp.text().await.unwrap_or_default()
            )),
        }
    }

    /// Merge-patch annotations onto a PV (adds/overwrites only the given keys).
    pub async fn annotate_pv(
        &self,
        name: &str,
        annotations: &BTreeMap<String, String>,
    ) -> Result<()> {
        let body = json!({ "metadata": { "annotations": annotations } });
        let resp = self
            .http
            .patch(self.pv_url(name))
            .bearer_auth(&self.token)
            .header("Content-Type", "application/merge-patch+json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!(
                "PATCH pv/{name}: {status} {}",
                resp.text().await.unwrap_or_default()
            ));
        }
        Ok(())
    }
}

/// `spec.csi.volumeHandle` of a PV document, if it is a CSI PV.
pub fn volume_handle(pv: &Value) -> Option<&str> {
    pv.pointer("/spec/csi/volumeHandle").and_then(Value::as_str)
}

/// Wait for the PV the provisioner creates for `pv_name` to show up with
/// our `volume_id` as its handle, then annotate it. Gives up quietly after
/// `wait` (the volume works either way; only the annotations are lost).
pub async fn annotate_pv_when_created(
    client: Client,
    pv_name: String,
    volume_id: String,
    annotations: BTreeMap<String, String>,
    wait: Duration,
) {
    let deadline = tokio::time::Instant::now() + wait;
    let mut interval = Duration::from_secs(1);
    loop {
        match client.get_pv(&pv_name).await {
            Ok(Some(pv)) => {
                if volume_handle(&pv) != Some(volume_id.as_str()) {
                    tracing::warn!(
                        "pv/{pv_name} has volumeHandle {:?}, not {volume_id}; not annotating",
                        volume_handle(&pv)
                    );
                    return;
                }
                match client.annotate_pv(&pv_name, &annotations).await {
                    Ok(()) => {
                        tracing::info!("annotated pv/{pv_name}: {:?}", annotations);
                        return;
                    }
                    Err(e) => tracing::warn!("annotating pv/{pv_name}: {e}"),
                }
            }
            Ok(None) => tracing::debug!("pv/{pv_name} not created yet"),
            Err(e) => tracing::warn!("looking up pv/{pv_name}: {e}"),
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!("gave up waiting for pv/{pv_name}; perf annotations not written");
            return;
        }
        tokio::time::sleep(interval).await;
        interval = (interval * 2).min(Duration::from_secs(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_from_csi_pv() {
        let pv = json!({"spec": {"csi": {"driver": "x", "volumeHandle": "abc"}}});
        assert_eq!(volume_handle(&pv), Some("abc"));
        assert_eq!(volume_handle(&json!({"spec": {"hostPath": {}}})), None);
    }
}
