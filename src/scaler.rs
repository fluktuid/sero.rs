use anyhow::{Context, Result};
use k8s_openapi::api::{apps::v1::Deployment, autoscaling::v1::Scale};
use kube::{
    api::{Api, Patch, PatchParams},
    Client,
};
use serde_json::json;
use tokio::time::{self, Duration};

pub async fn scale_deploy(name: &str, replicas: i32, ready_timeout: Duration) -> Result<()> {
    let client = Client::try_default().await?;
    let deploy: Api<Deployment> = Api::default_namespaced(client);
    let scale: Scale = serde_json::from_value(json!({
        "apiVersion": "autoscaling/v1",
        "kind": "Scale",
        "spec": {"replicas": replicas }
    }))?;

    // do a server-side apply where "sero" is the fieldManager
    let patch = Patch::Apply(&scale);
    let params = PatchParams::apply("sero");
    deploy.patch_scale(name, &params, &patch).await?;

    // wait until enough replicas are ready, this will periodically poll the apiserver
    // after a specified timeout, cancel and error
    let mut interval = time::interval(Duration::from_millis(100));
    time::timeout(ready_timeout, async move {
        while deploy
            .get_status(name)
            .await?
            .status
            .context("Deployment has no status o.O")?
            .ready_replicas
            .unwrap_or_default()
            < replicas
        {
            interval.tick().await;
        }
        Result::<()>::Ok(())
    })
    .await??;

    Ok(())
}
