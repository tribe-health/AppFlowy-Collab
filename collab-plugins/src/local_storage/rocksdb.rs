use std::ops::Deref;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use collab::preclude::CollabPlugin;
use collab_persistence::doc::YrsDocAction;
use collab_persistence::kv::rocks_kv::RocksCollabDB;
use y_sync::awareness::Awareness;
use yrs::{Transaction, TransactionMut};

#[derive(Clone)]
pub struct RocksdbDiskPlugin {
  uid: i64,
  db: Arc<RocksCollabDB>,
  did_load: Arc<AtomicBool>,
  /// the number of updates on disk when opening the document
  initial_update_count: Arc<AtomicU32>,
  update_count: Arc<AtomicU32>,
  config: CollabPersistenceConfig,
}

impl Deref for RocksdbDiskPlugin {
  type Target = Arc<RocksCollabDB>;

  fn deref(&self) -> &Self::Target {
    &self.db
  }
}

impl RocksdbDiskPlugin {
  pub fn new(uid: i64, db: Arc<RocksCollabDB>) -> Self {
    Self::new_with_config(uid, db, CollabPersistenceConfig::default())
  }

  pub fn new_with_config(
    uid: i64,
    db: Arc<RocksCollabDB>,
    config: CollabPersistenceConfig,
  ) -> Self {
    let initial_update_count = Arc::new(AtomicU32::new(0));
    let update_count = Arc::new(AtomicU32::new(0));
    let did_load = Arc::new(AtomicBool::new(false));
    Self {
      db,
      uid,
      did_load,
      initial_update_count,
      update_count,
      config,
    }
  }

  fn increase_count(&self) -> u32 {
    self.update_count.fetch_add(1, SeqCst)
  }
}

impl CollabPlugin for RocksdbDiskPlugin {
  fn init(&self, object_id: &str, txn: &mut TransactionMut) {
    let r_db_txn = self.db.read_txn();
    // Check the document is exist or not
    if r_db_txn.is_exist(self.uid, object_id) {
      // Safety: The document is exist, so it must be loaded successfully.
      match r_db_txn.load_doc(self.uid, object_id, txn) {
        Ok(update_count) => {
          self
            .initial_update_count
            .store(update_count, Ordering::SeqCst);
        },
        Err(e) => tracing::error!("🔴 load doc:{} failed: {}", object_id, e),
      }
      drop(r_db_txn);

      if self.config.flush_doc {
        let _ = self.db.with_write_txn(|w_db_txn| {
          w_db_txn.flush_doc(self.uid, object_id, txn)?;
          self.initial_update_count.store(0, Ordering::SeqCst);
          Ok(())
        });
      }
    } else {
      // Drop the read txn before write txn
      let result = self.db.with_write_txn(|w_db_txn| {
        w_db_txn.create_new_doc(self.uid, object_id, txn)?;
        Ok(())
      });

      if let Err(e) = result {
        tracing::error!("🔴 create doc for {:?} failed: {}", object_id, e)
      }
    }
  }

  fn did_init(&self, _awareness: &Awareness, _object_id: &str, _txn: &Transaction) {
    self.did_load.store(true, Ordering::SeqCst);
  }

  fn receive_update(&self, object_id: &str, _txn: &TransactionMut, update: &[u8]) {
    // Only push update if the doc is loaded
    if !self.did_load.load(Ordering::SeqCst) {
      return;
    }
    let _ = self.increase_count();
    // /Acquire a write transaction to ensure consistency
    let result = self.db.with_write_txn(|w_db_txn| {
      tracing::trace!("Receive {} update", object_id);
      let _ = w_db_txn.push_update(self.uid, object_id, update)?;
      Ok(())
    });

    if let Err(e) = result {
      tracing::error!("🔴Save update failed: {:?}", e);
    }
  }

  fn after_transaction(&self, _object_id: &str, _txn: &mut TransactionMut) {}
}

#[derive(Clone)]
pub struct CollabPersistenceConfig {
  /// Enable snapshot. Default is [false].
  pub enable_snapshot: bool,
  /// Generate a snapshot every N updates
  /// Default is 20. The value must be greater than 0.
  pub snapshot_per_update: u32,

  /// Flush the document. Default is [false].
  /// After flush the document, all updates will be removed and the document state vector that
  /// contains all the updates will be reset.
  pub(crate) flush_doc: bool,
}

impl CollabPersistenceConfig {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn enable_snapshot(mut self, enable_snapshot: bool) -> Self {
    self.enable_snapshot = enable_snapshot;
    self
  }

  pub fn snapshot_per_update(mut self, snapshot_per_update: u32) -> Self {
    debug_assert!(snapshot_per_update > 0);
    self.snapshot_per_update = snapshot_per_update;
    self
  }

  pub fn flush_doc(mut self, flush_doc: bool) -> Self {
    self.flush_doc = flush_doc;
    self
  }
}

impl Default for CollabPersistenceConfig {
  fn default() -> Self {
    Self {
      enable_snapshot: true,
      snapshot_per_update: 100,
      flush_doc: false,
    }
  }
}
