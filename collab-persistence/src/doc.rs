use std::fmt::Debug;

use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ReadTxn, StateVector, TransactionMut, Update};

use crate::keys::{
  doc_name_from_key, make_doc_end_key, make_doc_id_key, make_doc_start_key, make_doc_state_key,
  make_doc_update_key, make_state_vector_key, Clock, DocID, Key, DOC_SPACE, DOC_SPACE_OBJECT,
  DOC_SPACE_OBJECT_KEY,
};
use crate::kv::KVEntry;
use crate::kv::KVStore;
use crate::snapshot::SnapshotAction;
use crate::{
  get_id_for_key, get_last_update_key, insert_doc_update, make_doc_id_for_key, PersistenceError,
  TransactionMutExt,
};

impl<'a, T> YrsDocAction<'a> for T
where
  T: KVStore<'a>,
  PersistenceError: From<<Self as KVStore<'a>>::Error>,
{
}

pub trait YrsDocAction<'a>: KVStore<'a> + Sized
where
  PersistenceError: From<<Self as KVStore<'a>>::Error>,
{
  /// Create a new document with the given object id.
  fn create_new_doc<K: AsRef<[u8]> + ?Sized + Debug, T: ReadTxn>(
    &self,
    uid: i64,
    object_id: &K,
    txn: &T,
  ) -> Result<(), PersistenceError> {
    if self.is_exist(uid, object_id) {
      tracing::warn!("🟡{:?} already exist", object_id);
    }
    let doc_id = get_or_create_did(uid, self, object_id.as_ref())?;
    tracing::trace!(
      "[🦀Collab] => [{}:{:?}]: new doc:{}",
      doc_id,
      object_id,
      doc_id
    );
    let doc_state = txn.encode_diff_v1(&StateVector::default());
    let sv = txn.state_vector().encode_v1();
    let doc_state_key = make_doc_state_key(doc_id);
    let sv_key = make_state_vector_key(doc_id);

    tracing::trace!(
      "[🦀Collab] => [{}:{:?}] insert doc state: {:?}",
      doc_id,
      object_id,
      doc_state_key
    );
    self.insert(doc_state_key, doc_state)?;
    self.insert(sv_key, sv)?;

    Ok(())
  }

  /// Load the document from the database and apply the updates to the transaction.
  /// After loading the document, it will delete the document state vec and updates and
  /// insert the new document state.
  fn flush_doc<K: AsRef<[u8]> + ?Sized + Debug, T: ReadTxn>(
    &self,
    uid: i64,
    object_id: &K,
    txn: &T,
  ) -> Result<(), PersistenceError> {
    let doc_id = get_or_create_did(uid, self, object_id)?;
    tracing::trace!("[🦀Collab] => [{}:{:?}]: flush doc", doc_id, object_id);

    let doc_state = txn.encode_state_as_update_v1(&StateVector::default());
    let sv = txn.state_vector().encode_v1();

    let doc_state_key = make_doc_state_key(doc_id);
    let sv_key = make_state_vector_key(doc_id);

    tracing::trace!(
      "[🦀Collab] => [{}:{:?}] insert doc state: {:?} : {}",
      doc_id,
      object_id,
      doc_state_key.as_ref(),
      doc_state.len(),
    );
    // Insert new doc state and state vector
    self.insert(doc_state_key, doc_state)?;
    self.insert(sv_key, sv)?;

    // Remove the updates
    let start = make_doc_start_key(doc_id);
    let end = make_doc_end_key(doc_id);
    self.remove_range(start.as_ref(), end.as_ref())?;

    Ok(())
  }

  fn is_exist<K: AsRef<[u8]> + ?Sized + Debug>(&self, collab_id: i64, object_id: &K) -> bool {
    get_doc_id(collab_id, self, object_id).is_some()
  }

  /// Load the document from the database and apply the updates to the transaction.
  /// It will try to load the document in these two ways:
  ///   1. D = document state + updates
  ///   2. D = document state + snapshot + updates
  ///
  /// Return the number of updates
  fn load_doc<K: AsRef<[u8]> + ?Sized + Debug>(
    &self,
    uid: i64,
    object_id: &K,
    txn: &mut TransactionMut,
  ) -> Result<u32, PersistenceError> {
    let mut update_count = 0;

    if let Some(doc_id) = get_doc_id(uid, self, object_id) {
      let doc_state_key = make_doc_state_key(doc_id);
      if let Some(doc_state) = self.get(doc_state_key.as_ref())? {
        // Load the doc state
        if let Err(e) = Update::decode_v1(doc_state.as_ref())
          .map_err(PersistenceError::Yrs)
          .and_then(|update| txn.try_apply_update(update))
        {
          tracing::error!("🔴{:?} apply doc state error: {}", object_id, e)
        }

        // If the enable_snapshot is true, we will try to load the snapshot.
        let update_start = make_doc_update_key(doc_id, 0).to_vec();
        let update_end = make_doc_update_key(doc_id, Clock::MAX);
        // tracing::trace!(
        //   "[🦀Collab] => [{}-{:?}]: Get update from {:?} to {:?}",
        //   doc_id,
        //   object_id,
        //   &update_start,
        //   update_end.as_ref(),
        // );

        // Load the updates
        let encoded_updates = self.range(update_start.as_ref()..update_end.as_ref())?;
        for encoded_update in encoded_updates {
          // Decode the update and apply it to the transaction. If the update is invalid, we will
          // remove the update and the following updates.
          if let Err(e) = Update::decode_v1(encoded_update.value())
            .map_err(PersistenceError::Yrs)
            .and_then(|update| txn.try_apply_update(update))
          {
            tracing::error!("🔴{:?} apply update error: {}", object_id, e);
            self.remove_range(encoded_update.key().as_ref(), update_end.as_ref())?;
            break;
          }
          update_count += 1;
        }
      } else {
        tracing::error!(
          "🔴collab => [{}-{:?}]: the doc state should not be empty",
          doc_id,
          object_id
        );
      }
      Ok(update_count)
    } else {
      tracing::trace!("[🦀Collab] => {:?} not exist", object_id);
      Err(PersistenceError::DocumentNotExist)
    }
  }

  // fn load_latest_snapshot<K: AsRef<[u8]> + ?Sized + Debug>(
  //   &self,
  //   uid: i64,
  //   object_id: &K,
  //   txn: &mut TransactionMut,
  // ) -> Option<Vec<u8>> {
  //   let snapshot_id = get_snapshot_id(uid, self, object_id)?;
  //   let snapshot = self.get_last_snapshot_update(snapshot_id)?;
  //   // Decode the data of the snapshot and apply it to the transaction.
  //   // If the snapshot is invalid, the snapshot will be deleted. After delete the snapshot,
  //   // try to load the next latest snapshot.
  //   match Update::decode_v1(&snapshot.data) {
  //     Ok(update) => match txn.try_apply_update(update) {
  //       Ok(_) => {},
  //       Err(e) => {
  //         tracing::error!(
  //           "🔴{:?} apply snapshot error: {}. try to load next snapshot",
  //           object_id,
  //           e
  //         );
  //         self.delete_last_snapshot_update(snapshot_id);
  //         return self.load_latest_snapshot(uid, object_id, txn);
  //       },
  //     },
  //     Err(_) => {
  //       self.delete_last_snapshot_update(snapshot_id);
  //       tracing::error!(
  //         "🔴{:?} decode snapshot error, try to load next snapshot",
  //         object_id
  //       );
  //       return self.load_latest_snapshot(uid, object_id, txn);
  //     },
  //   }
  //   Some(snapshot.update_key)
  // }

  /// Push an update to the persistence
  fn push_update<K: AsRef<[u8]> + ?Sized + Debug>(
    &self,
    uid: i64,
    object_id: &K,
    update: &[u8],
  ) -> Result<Vec<u8>, PersistenceError> {
    match get_doc_id(uid, self, object_id.as_ref()) {
      None => {
        tracing::error!(
          "🔴Insert update failed. Can't find the doc for {:?}",
          object_id
        );
        Err(PersistenceError::DocumentNotExist)
      },
      Some(doc_id) => insert_doc_update(self, doc_id, object_id, update.to_vec()),
    }
  }

  /// Delete the updates that prior to the given key. The given key is not included.
  fn delete_updates_to<K: AsRef<[u8]> + ?Sized + Debug>(
    &self,
    uid: i64,
    object_id: &K,
    end: &[u8],
  ) -> Result<(), PersistenceError> {
    if let Some(doc_id) = get_doc_id(uid, self, object_id) {
      let start = make_doc_update_key(doc_id, 0);
      self.remove_range(start.as_ref(), end.as_ref())?;
    }
    Ok(())
  }

  /// Delete the document from the persistence
  /// This will remove all the updates and the document state
  fn delete_doc<K: AsRef<[u8]> + ?Sized + Debug>(
    &self,
    uid: i64,
    object_id: &K,
  ) -> Result<(), PersistenceError> {
    if let Some(did) = get_doc_id(uid, self, object_id) {
      tracing::trace!("[🦀Collab] => [{}] delete {:?} doc", did, object_id);
      let key = make_doc_id_key(&uid.to_be_bytes(), object_id.as_ref());
      let _ = self.remove(key.as_ref());

      // Delete the updates
      let start = make_doc_start_key(did);
      let end = make_doc_end_key(did);
      self.remove_range(start.as_ref(), end.as_ref())?;

      // Delete the document state and the state vector
      let doc_state_key = make_doc_state_key(did);
      let sv_key = make_state_vector_key(did);
      let _ = self.remove(doc_state_key.as_ref());
      let _ = self.remove(sv_key.as_ref());

      // Delete the snapshot
      self.delete_all_snapshots(uid, object_id)?;
    }
    Ok(())
  }

  fn get_all_docs(
    &self,
  ) -> Result<NameIter<<Self as KVStore<'a>>::Range, <Self as KVStore<'a>>::Entry>, PersistenceError>
  {
    let from = Key::from_const([DOC_SPACE, DOC_SPACE_OBJECT]);
    let to = Key::from_const([DOC_SPACE, DOC_SPACE_OBJECT_KEY]);
    let iter = self.range(from.as_ref()..to.as_ref())?;
    Ok(NameIter { iter })
  }

  /// Return all the updates for the given document
  fn get_decoded_v1_updates<K: AsRef<[u8]> + ?Sized>(
    &self,
    uid: i64,
    object_id: &K,
  ) -> Result<Vec<Update>, PersistenceError> {
    if let Some(doc_id) = get_doc_id(uid, self, object_id) {
      let start = make_doc_update_key(doc_id, 0);
      let end = make_doc_update_key(doc_id, Clock::MAX);

      let mut updates = vec![];
      if let Ok(encoded_updates) = self.range(start.as_ref()..=end.as_ref()) {
        for encoded_update in encoded_updates {
          updates.push(Update::decode_v1(encoded_update.value())?);
        }
      }
      Ok(updates)
    } else {
      Err(PersistenceError::DocumentNotExist)
    }
  }

  fn get_doc_last_update_key<K: AsRef<[u8]> + ?Sized>(
    &self,
    uid: i64,
    object_id: &K,
  ) -> Option<Key<16>> {
    let doc_id = get_doc_id(uid, self, object_id)?;
    get_last_update_key(self, doc_id, make_doc_update_key).ok()
  }

  /// Return the number of updates for the given document
  fn number_of_updates<K: AsRef<[u8]> + ?Sized>(&self, uid: i64, object_id: &K) -> usize {
    if let Some(doc_id) = get_doc_id(uid, self, object_id) {
      let start = make_doc_update_key(doc_id, 0);
      let end = make_doc_update_key(doc_id, Clock::MAX);
      self
        .range(start.as_ref()..=end.as_ref())
        .map(|r| r.count())
        .unwrap_or(0)
    } else {
      0
    }
  }
}

/// Get or create a document id for the given object id.
fn get_or_create_did<'a, K, S>(
  uid: i64,
  store: &S,
  object_id: &K,
) -> Result<DocID, PersistenceError>
where
  S: KVStore<'a>,
  PersistenceError: From<<S as KVStore<'a>>::Error>,
  K: AsRef<[u8]> + ?Sized + Debug,
{
  if let Some(did) = get_doc_id(uid, store, object_id.as_ref()) {
    Ok(did)
  } else {
    let key = make_doc_id_key(&uid.to_be_bytes(), object_id.as_ref());
    let new_did = make_doc_id_for_key(store, key)?;
    Ok(new_did)
  }
}

fn get_doc_id<'a, K, S>(collab_id: i64, store: &S, object_id: &K) -> Option<DocID>
where
  S: KVStore<'a>,
  K: AsRef<[u8]> + ?Sized,
{
  let collab_id_bytes = &collab_id.to_be_bytes();
  let key = make_doc_id_key(collab_id_bytes, object_id.as_ref());
  get_id_for_key(store, key)
}

pub struct NameIter<I, E>
where
  I: Iterator<Item = E>,
  E: KVEntry,
{
  iter: I,
}

impl<I, E> Iterator for NameIter<I, E>
where
  I: Iterator<Item = E>,
  E: KVEntry,
{
  type Item = String;

  fn next(&mut self) -> Option<Self::Item> {
    let entry = self.iter.next()?;
    let content = doc_name_from_key(entry.key());
    Some(String::from_utf8_lossy(content).to_string())
  }
}
