//use k8s_openapi::api::{discovery::v1:Endpoint, autoscaling::v1::Scale};
use k8s_openapi::{api::discovery::v1::{EndpointSlice, Endpoint, EndpointPort}, Metadata};
use kube::{
  api::{Api, Patch, PatchParams, ObjectMeta, PostParams, ListParams},
  Client,
};
use serde_json::{json, from_value};
use anyhow::{Result, Ok};
use std::{collections::BTreeMap, str::FromStr};
use tracing::{error, info};
use local_ip_address::local_ip;
use json_patch::PatchOperation;

pub struct Slice {
  svc_name: String,
  deploy_name: String,
  client: Client,
}

impl Slice {
  /// Create a new `Semaphore`
  pub fn new(svc_name: &str, deploy_name: &str, client: &Client) -> Slice {
    Slice {
      svc_name:    String::from_str(svc_name).unwrap(),
      deploy_name: String::from_str(deploy_name).unwrap(),
      client:      client.clone(),
    }
  }

  async fn slice_name(&self) -> Result<Option<String>> {
    info!("slice name xxx");
    info!("creating got client");
    let slice: Api<EndpointSlice> = Api::default_namespaced(self.client.clone());
    info!("creating got slice");
    let svc_name = &self.svc_name;
    let lp = ListParams {
      label_selector: Some(format!("kubernetes.io/service-name={svc_name}")),
      ..Default::default()
    };
    info!("list objects");
    let obj_list = slice.list(&lp).await;
    if obj_list.is_err() {
      let e = obj_list.unwrap_err();
      error!("can't get object list: {e}");
      return Ok(None);
    }
    let list = obj_list.unwrap();
    for n in &list.items {
      let name = n.clone().metadata.name.unwrap();
      info!("{name}");
      let annotations_opt = n.clone().metadata.annotations;
      if annotations_opt.is_none() {
        continue;
      }
      let annotations = annotations_opt.unwrap();
      let v = annotations.get("sero/target-deployment");
      match v {
        Some(ve) => {
            if *ve == self.deploy_name {
              info!("slice name found: {name}");
              return Ok(Some(name))
            }
          },
          None => {},
      }
    }
    info!("no slice name found");
    return Ok(None);
  }

  pub async fn apply_slice(&self) -> Result<String>  {
    match self.slice_name().await? {
        Some(v) => {
          info!("slice found: {v}");
          return Ok(v)
        },
        None => {},
    }
    info!("creating slice");
    let slice: Api<EndpointSlice> = Api::default_namespaced(self.client.clone());
    let ep = Endpoint {
      addresses: vec![local_ip().unwrap().to_string()],
      ..Default::default()
    };
    let epp = EndpointPort {
      name: Some("http".to_owned()),
      protocol: Some("TCP".to_owned()),
      port: Some(8080),
      app_protocol: Some("http".to_owned()),
    };
    let metadata = ObjectMeta {
      annotations: Some(BTreeMap::from([
        ("sero/target-deployment".to_owned(), self.deploy_name.clone()),
      ])),
      generate_name: Some("sero-".to_owned()),
      labels: Some(BTreeMap::from([
        ("kubernetes.io/service-name".to_owned(), self.svc_name.clone()),
      ])),
      ..Default::default()
    };
    let ep_slice: EndpointSlice = EndpointSlice {
      address_type: "IPv4".to_owned(),
      endpoints: vec![ep],
      metadata: metadata,
      ports: Some(vec![epp]),
    };

    let pp = PostParams::default();
    let res = slice.create(&pp, &ep_slice).await;
    if res.is_ok() {
      info!("Successfully applied endpointslice");
      let name = res.unwrap().metadata().name.as_ref().unwrap().clone();
      return Ok(name);
    } else {
      let e = res.unwrap_err();
      error!("failed applying endpoint slice: {e}");
      return Err(e.into());
    }
  }

  pub async fn append_ep_to_svc(&self) -> Result<()>  {
    let name: String;
    match self.slice_name().await? {
        Some(e) => {name = e.clone()},
        None => {
          name = self.apply_slice().await.unwrap();
        },
    }

    let slice: Api<EndpointSlice> = Api::default_namespaced(self.client.clone());
    // "kubernetes.io/service-name"
    let p: Vec<PatchOperation> = from_value(json!([{
      "op": "add",
      "path": "/metadata/labels/kubernetes.io~1service-name",
      "value": self.svc_name
    },])).unwrap();

    // do a server-side apply where "sero" is the fieldManager
    let json_patch = json_patch::Patch(p);
    let patch = Patch::Json::<()>(json_patch);
    let params = PatchParams::apply("sero");
    let result = slice.patch_metadata(&name, &params, &patch).await;
    if result.is_err() {
      let e = result.unwrap_err();
      error!("failed appending endpoint slice to service: {e}");
    }

    Ok(())
  }

  pub async fn remove_ep_from_svc(&self) -> Result<()>  {
    let name: String;
    match self.slice_name().await? {
        Some(e) => {name = e.clone()},
        None => {
          name = self.apply_slice().await.unwrap();
        },
    }

    let slice: Api<EndpointSlice> = Api::default_namespaced(self.client.clone());
    let p: Vec<PatchOperation> = from_value(json!([
      { "apiVersion": "discovery.k8s.io/v1",
        "op": "remove", "path": "/metadata/labels/kubernetes.io~1service-name" },
    ])).unwrap();

    // do a server-side apply where "sero" is the fieldManager
    let json_patch = json_patch::Patch(p);
    let patch = Patch::Json::<()>(json_patch);
    let params = PatchParams::apply(&self.svc_name);
    let result = slice.patch(&name, &params, &patch).await;
    if result.is_err() {
      let e = result.unwrap_err();
      error!("failed removing endpoint slice from service: {e}");
    }

    Ok(())
  }
}
