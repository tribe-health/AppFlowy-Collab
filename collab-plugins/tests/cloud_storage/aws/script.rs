use crate::setup_log;
use collab::core::collab::MutexCollab;
use collab::core::origin::{CollabClient, CollabOrigin};
use collab::preclude::Collab;
use collab_plugins::cloud_storage::aws::{get_aws_remote_doc, AWSDynamoDBPlugin};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub enum TestScript {
  CreateCollab {
    uid: i64,
    object_id: String,
    sync_per_secs: u64,
  },
  ModifyCollab {
    uid: i64,
    object_id: String,
    f: Box<dyn FnOnce(&Collab)>,
  },
  Wait {
    secs: u64,
  },
  AssertLocal {
    uid: i64,
    object_id: String,
    expected: Value,
  },
  AssertRemote {
    object_id: String,
    expected: Value,
  },
}

pub struct AWSStorageTest {
  pub collab_by_id: HashMap<String, Arc<MutexCollab>>,
}

impl AWSStorageTest {
  pub fn new() -> Self {
    setup_log();
    Self {
      collab_by_id: HashMap::new(),
    }
  }

  pub async fn run_script(&mut self, script: TestScript) {
    match script {
      TestScript::CreateCollab {
        uid,
        object_id,
        sync_per_secs,
      } => {
        let origin = CollabOrigin::Client(CollabClient::new(uid, "1"));
        let local_collab = Arc::new(MutexCollab::new(origin, &object_id, vec![]));
        let plugin = AWSDynamoDBPlugin::new(
          object_id.clone(),
          local_collab.clone(),
          sync_per_secs,
          test_region(),
        );
        local_collab.lock().add_plugin(Arc::new(plugin));
        local_collab.initial();
        self
          .collab_by_id
          .insert(make_id(uid, &object_id), local_collab);
      },
      TestScript::ModifyCollab { uid, object_id, f } => {
        let collab = self
          .collab_by_id
          .get(&make_id(uid, &object_id))
          .unwrap()
          .lock();
        f(&collab);
      },
      TestScript::Wait { secs } => {
        tokio::time::sleep(Duration::from_secs(secs)).await;
      },
      TestScript::AssertLocal {
        uid,
        object_id,
        expected,
      } => {
        let id = format!("{}-{}", uid, object_id);
        let collab = self.collab_by_id.get(&id).unwrap().lock();
        assert_json_diff::assert_json_eq!(collab.to_json_value(), expected,);
      },
      TestScript::AssertRemote {
        object_id,
        expected,
      } => {
        let collab = get_aws_remote_doc(&object_id, test_region()).await;
        let json = collab.lock().to_json_value();
        assert_json_diff::assert_json_eq!(json, expected,);
      },
    }
  }

  pub async fn run_scripts(&mut self, scripts: Vec<TestScript>) {
    for script in scripts {
      self.run_script(script).await;
    }
  }
}
fn test_region() -> String {
  "ap-southeast-2".to_string()
}

pub fn make_id(uid: i64, object_id: &str) -> String {
  format!("{}-{}", uid, object_id)
}
