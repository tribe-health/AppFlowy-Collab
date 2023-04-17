use collab_persistence::CollabKV;
use std::sync::Once;
use tempfile::TempDir;
use tracing_subscriber::{fmt::Subscriber, util::SubscriberInitExt, EnvFilter};

pub fn db() -> CollabKV {
  static START: Once = Once::new();
  START.call_once(|| {
    std::env::set_var("RUST_LOG", "collab_persistence=trace");
    let subscriber = Subscriber::builder()
      .with_env_filter(EnvFilter::from_default_env())
      .with_ansi(true)
      .finish();
    subscriber.try_init().unwrap();
  });

  let tempdir = TempDir::new().unwrap();
  let path = tempdir.into_path();
  CollabKV::open(path).unwrap()
}
