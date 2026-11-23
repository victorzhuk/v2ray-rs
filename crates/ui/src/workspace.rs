use std::sync::{Arc, Mutex, RwLock, mpsc};

use uuid::Uuid;
use v2ray_rs_core::models::{AppSettings, ManualNode, RoutingRuleSet, Subscription};
use v2ray_rs_core::persistence::{self, AppPaths, PersistenceError};
use v2ray_rs_core::resolve::LatencySnapshot;

type WriteResult = Result<(), PersistenceError>;
type WriteJob = Box<dyn FnOnce(&AppPaths) -> WriteResult + Send>;

/// Serializes every workspace write on one background thread. Callers wait for
/// its acknowledgement, so a successful `save_*` still means the bytes reached
/// disk while the UI thread never performs the write itself.
struct Writer {
    tx: mpsc::Sender<(WriteJob, mpsc::SyncSender<WriteResult>)>,
}

impl Writer {
    fn new(paths: AppPaths) -> Self {
        let (tx, rx) = mpsc::channel::<(WriteJob, mpsc::SyncSender<WriteResult>)>();
        std::thread::Builder::new()
            .name("workspace-writer".into())
            .spawn(move || {
                while let Ok((job, reply)) = rx.recv() {
                    let _ = reply.send(job(&paths));
                }
            })
            .expect("spawn workspace writer");
        Self { tx }
    }

    fn persist<F>(&self, job: F) -> WriteResult
    where
        F: FnOnce(&AppPaths) -> WriteResult + Send + 'static,
    {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx
            .send((Box::new(job), reply_tx))
            .map_err(|_| PersistenceError::CorruptConfig("workspace writer stopped".into()))?;
        reply_rx
            .recv()
            .map_err(|_| PersistenceError::CorruptConfig("workspace writer stopped".into()))?
    }
}

#[derive(Clone, Default)]
struct Snapshot {
    settings: Option<Arc<AppSettings>>,
    subscriptions: Option<Arc<Vec<Subscription>>>,
    manual_nodes: Option<Arc<Vec<ManualNode>>>,
    routing_rules: Option<Arc<RoutingRuleSet>>,
    latency: Option<Arc<LatencySnapshot>>,
}

#[derive(Clone)]
pub struct WorkspaceStore {
    paths: AppPaths,
    lock: Arc<Mutex<()>>,
    snapshot: Arc<RwLock<Snapshot>>,
    writer: Arc<Writer>,
}

impl WorkspaceStore {
    pub fn new(paths: AppPaths) -> Self {
        let writer = Arc::new(Writer::new(paths.clone()));
        Self {
            paths,
            lock: Arc::new(Mutex::new(())),
            snapshot: Arc::new(RwLock::new(Snapshot::default())),
            writer,
        }
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn load_settings(&self) -> Result<AppSettings, PersistenceError> {
        Ok((*self.settings_snapshot()?).clone())
    }

    pub fn settings_snapshot(&self) -> Result<Arc<AppSettings>, PersistenceError> {
        let _guard = self.lock();
        self.settings_locked()
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), PersistenceError> {
        let _guard = self.lock();
        self.persist_settings(settings.clone())
    }

    pub fn update_settings<F>(&self, mutate: F) -> Result<AppSettings, PersistenceError>
    where
        F: FnOnce(&mut AppSettings),
    {
        let _guard = self.lock();
        let mut settings = (*self.settings_locked()?).clone();
        mutate(&mut settings);
        self.persist_settings(settings.clone())?;
        Ok(settings)
    }

    pub fn load_subscriptions(&self) -> Result<Vec<Subscription>, PersistenceError> {
        Ok((*self.subscriptions_snapshot()?).clone())
    }

    pub fn subscriptions_snapshot(&self) -> Result<Arc<Vec<Subscription>>, PersistenceError> {
        let _guard = self.lock();
        self.subscriptions_locked()
    }

    pub fn save_subscriptions(
        &self,
        subscriptions: &[Subscription],
    ) -> Result<(), PersistenceError> {
        let _guard = self.lock();
        self.persist_subscriptions(subscriptions.to_vec())
    }

    pub fn update_subscriptions<F, R>(&self, mutate: F) -> Result<R, PersistenceError>
    where
        F: FnOnce(&mut Vec<Subscription>) -> R,
    {
        let _guard = self.lock();
        let mut subscriptions = (*self.subscriptions_locked()?).clone();
        let result = mutate(&mut subscriptions);
        self.persist_subscriptions(subscriptions)?;
        Ok(result)
    }

    pub fn get_subscription(&self, id: Uuid) -> Result<Option<Subscription>, PersistenceError> {
        let subscriptions = self.subscriptions_snapshot()?;
        Ok(subscriptions
            .iter()
            .find(|subscription| subscription.id == id)
            .cloned())
    }

    pub fn load_manual_nodes(&self) -> Result<Vec<ManualNode>, PersistenceError> {
        Ok((*self.manual_nodes_snapshot()?).clone())
    }

    pub fn manual_nodes_snapshot(&self) -> Result<Arc<Vec<ManualNode>>, PersistenceError> {
        let _guard = self.lock();
        self.manual_nodes_locked()
    }

    pub fn load_manual_nodes_or_default(&self) -> Vec<ManualNode> {
        match self.load_manual_nodes() {
            Ok(nodes) => nodes,
            Err(err) => {
                log::warn!("load manual nodes: {err}");
                Vec::new()
            }
        }
    }

    pub fn save_manual_nodes(&self, nodes: &[ManualNode]) -> Result<(), PersistenceError> {
        let _guard = self.lock();
        self.persist_manual_nodes(nodes.to_vec())
    }

    pub fn update_manual_nodes<F, R>(&self, mutate: F) -> Result<R, PersistenceError>
    where
        F: FnOnce(&mut Vec<ManualNode>) -> R,
    {
        let _guard = self.lock();
        let mut nodes = (*self.manual_nodes_locked()?).clone();
        let result = mutate(&mut nodes);
        self.persist_manual_nodes(nodes)?;
        Ok(result)
    }

    pub fn load_routing_rules(&self) -> Result<RoutingRuleSet, PersistenceError> {
        Ok((*self.routing_rules_snapshot()?).clone())
    }

    pub fn routing_rules_snapshot(&self) -> Result<Arc<RoutingRuleSet>, PersistenceError> {
        let _guard = self.lock();
        self.routing_rules_locked()
    }

    pub fn save_routing_rules(&self, rules: &RoutingRuleSet) -> Result<(), PersistenceError> {
        let _guard = self.lock();
        self.persist_routing_rules(rules.clone())
    }

    pub fn update_routing_rules<F, R>(&self, mutate: F) -> Result<R, PersistenceError>
    where
        F: FnOnce(&mut RoutingRuleSet) -> R,
    {
        let _guard = self.lock();
        let mut rules = (*self.routing_rules_locked()?).clone();
        let result = mutate(&mut rules);
        self.persist_routing_rules(rules)?;
        Ok(result)
    }

    pub fn load_latency_snapshot(&self) -> Result<LatencySnapshot, PersistenceError> {
        Ok((*self.latency_snapshot()?).clone())
    }

    pub fn latency_snapshot(&self) -> Result<Arc<LatencySnapshot>, PersistenceError> {
        let _guard = self.lock();
        self.latency_locked()
    }

    pub fn save_latency_snapshot(
        &self,
        snapshot: &LatencySnapshot,
    ) -> Result<(), PersistenceError> {
        let _guard = self.lock();
        self.persist_latency(snapshot.clone())
    }

    /// Mutates and persists the latency snapshot.
    ///
    /// The mutate closure runs while the store mutex is held: it must not call
    /// other `WorkspaceStore` methods — the lock is non-reentrant and would
    /// self-deadlock.
    pub fn update_latency_snapshot<R>(
        &self,
        mutate: impl FnOnce(&mut LatencySnapshot) -> R,
    ) -> Result<R, PersistenceError> {
        let _guard = self.lock();
        let mut latency = (*self.latency_locked()?).clone();
        let result = mutate(&mut latency);
        self.persist_latency(latency)?;
        Ok(result)
    }

    pub fn reset_subscriptions(&self) -> Result<(), std::io::Error> {
        let _guard = self.lock();
        remove_if_exists(self.paths.subscriptions_path())?;
        self.write_snapshot(|snapshot| snapshot.subscriptions = Some(Arc::new(Vec::new())));
        Ok(())
    }

    pub fn reset_manual_nodes(&self) -> Result<(), std::io::Error> {
        let _guard = self.lock();
        remove_if_exists(self.paths.custom_nodes_path())?;
        self.write_snapshot(|snapshot| snapshot.manual_nodes = Some(Arc::new(Vec::new())));
        Ok(())
    }

    pub fn reset_routing_rules(&self) -> Result<(), PersistenceError> {
        let _guard = self.lock();
        self.persist_routing_rules(RoutingRuleSet::new())
    }

    /// Removes `settings.toml` on the writer thread and seeds the snapshot
    /// with defaults, so every later reader sees the reset. A missing file
    /// resets identically to a present one. Returns the defaults callers
    /// should adopt.
    pub fn reset_settings(&self) -> Result<Arc<AppSettings>, PersistenceError> {
        let _guard = self.lock();
        self.writer.persist(|paths| {
            remove_if_exists(paths.settings_path()).map_err(PersistenceError::from)
        })?;
        let settings = Arc::new(AppSettings::default());
        self.write_snapshot(|snapshot| snapshot.settings = Some(settings.clone()));
        Ok(settings)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn read_snapshot(&self) -> Snapshot {
        self.snapshot
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn write_snapshot(&self, mutate: impl FnOnce(&mut Snapshot)) {
        let mut snapshot = self
            .snapshot
            .write()
            .unwrap_or_else(|error| error.into_inner());
        mutate(&mut snapshot);
    }

    fn settings_locked(&self) -> Result<Arc<AppSettings>, PersistenceError> {
        if let Some(settings) = self.read_snapshot().settings {
            return Ok(settings);
        }
        let settings = Arc::new(persistence::load_settings(&self.paths)?);
        self.write_snapshot(|snapshot| snapshot.settings = Some(settings.clone()));
        Ok(settings)
    }

    fn subscriptions_locked(&self) -> Result<Arc<Vec<Subscription>>, PersistenceError> {
        if let Some(subscriptions) = self.read_snapshot().subscriptions {
            return Ok(subscriptions);
        }
        let subscriptions = Arc::new(persistence::load_subscriptions(&self.paths)?);
        self.write_snapshot(|snapshot| snapshot.subscriptions = Some(subscriptions.clone()));
        Ok(subscriptions)
    }

    fn manual_nodes_locked(&self) -> Result<Arc<Vec<ManualNode>>, PersistenceError> {
        if let Some(nodes) = self.read_snapshot().manual_nodes {
            return Ok(nodes);
        }
        let nodes = Arc::new(persistence::load_manual_nodes(&self.paths)?);
        self.write_snapshot(|snapshot| snapshot.manual_nodes = Some(nodes.clone()));
        Ok(nodes)
    }

    fn routing_rules_locked(&self) -> Result<Arc<RoutingRuleSet>, PersistenceError> {
        if let Some(rules) = self.read_snapshot().routing_rules {
            return Ok(rules);
        }
        let rules = Arc::new(persistence::load_routing_rules(&self.paths)?);
        self.write_snapshot(|snapshot| snapshot.routing_rules = Some(rules.clone()));
        Ok(rules)
    }

    fn latency_locked(&self) -> Result<Arc<LatencySnapshot>, PersistenceError> {
        if let Some(latency) = self.read_snapshot().latency {
            return Ok(latency);
        }
        let latency = Arc::new(persistence::load_latency_snapshot(&self.paths)?);
        self.write_snapshot(|snapshot| snapshot.latency = Some(latency.clone()));
        Ok(latency)
    }

    fn persist_settings(&self, settings: AppSettings) -> WriteResult {
        let write = settings.clone();
        self.writer.persist(move |paths| {
            persistence::save_settings(paths, &write)?;
            Ok(())
        })?;
        self.write_snapshot(|snapshot| snapshot.settings = Some(Arc::new(settings)));
        Ok(())
    }

    fn persist_subscriptions(&self, subscriptions: Vec<Subscription>) -> WriteResult {
        let write = subscriptions.clone();
        self.writer.persist(move |paths| {
            persistence::save_subscriptions(paths, &write)?;
            Ok(())
        })?;
        self.write_snapshot(|snapshot| {
            snapshot.subscriptions = Some(Arc::new(subscriptions));
        });
        Ok(())
    }

    fn persist_manual_nodes(&self, nodes: Vec<ManualNode>) -> WriteResult {
        let write = nodes.clone();
        self.writer.persist(move |paths| {
            persistence::save_manual_nodes(paths, &write)?;
            Ok(())
        })?;
        self.write_snapshot(|snapshot| snapshot.manual_nodes = Some(Arc::new(nodes)));
        Ok(())
    }

    fn persist_routing_rules(&self, rules: RoutingRuleSet) -> WriteResult {
        let write = rules.clone();
        self.writer.persist(move |paths| {
            persistence::save_routing_rules(paths, &write)?;
            Ok(())
        })?;
        self.write_snapshot(|snapshot| snapshot.routing_rules = Some(Arc::new(rules)));
        Ok(())
    }

    fn persist_latency(&self, latency: LatencySnapshot) -> WriteResult {
        let write = latency.clone();
        self.writer.persist(move |paths| {
            persistence::save_latency_snapshot(paths, &write)?;
            Ok(())
        })?;
        self.write_snapshot(|snapshot| snapshot.latency = Some(Arc::new(latency)));
        Ok(())
    }
}

fn remove_if_exists(path: std::path::PathBuf) -> Result<(), std::io::Error> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use v2ray_rs_core::profile::AppProfile;

    fn store() -> (tempfile::TempDir, WorkspaceStore) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        paths.ensure_dirs().unwrap();
        (tmp, WorkspaceStore::new(paths))
    }

    #[test]
    fn reads_share_one_snapshot_until_a_write_replaces_it() {
        let (_tmp, store) = store();
        let first = store.settings_snapshot().unwrap();
        let second = store.settings_snapshot().unwrap();
        assert!(Arc::ptr_eq(&first, &second));

        let mut settings = (*first).clone();
        settings.http_port = 2091;
        store.save_settings(&settings).unwrap();

        let third = store.settings_snapshot().unwrap();
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(third.http_port, 2091);
    }

    #[test]
    fn concurrent_updates_do_not_lose_writes() {
        let (_tmp, store) = store();
        let workers = 8;
        let per_worker = 25;
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for _ in 0..per_worker {
                        store
                            .update_subscriptions(|subscriptions| {
                                subscriptions.push(Subscription::new_from_url(
                                    format!("worker-{worker}"),
                                    "https://example.com/sub",
                                ));
                            })
                            .unwrap();
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(
            store.load_subscriptions().unwrap().len(),
            workers * per_worker
        );
        assert_eq!(
            persistence::load_subscriptions(store.paths())
                .unwrap()
                .len(),
            workers * per_worker
        );
    }

    #[test]
    fn concurrent_latency_updates_do_not_lose_writes() {
        let (_tmp, store) = store();
        let workers = 8;
        let per_worker = 25;
        let node_ref =
            |worker: usize, i: usize| v2ray_rs_core::models::ConnectionNodeRef::Subscription {
                subscription_id: Uuid::from_u128(worker as u128),
                node_id: Uuid::from_u128(i as u128),
            };
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for i in 0..per_worker {
                        store
                            .update_latency_snapshot(|snapshot| {
                                snapshot.upsert(node_ref(worker, i), 42, chrono::Utc::now());
                            })
                            .unwrap();
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let snapshot = store.load_latency_snapshot().unwrap();
        let persisted = persistence::load_latency_snapshot(store.paths()).unwrap();
        for worker in 0..workers {
            for i in 0..per_worker {
                let expected = node_ref(worker, i);
                assert!(
                    snapshot.get(expected).is_some(),
                    "in-memory snapshot missing worker {worker} sample {i}"
                );
                assert!(
                    persisted.get(expected).is_some(),
                    "latency_snapshot.json missing worker {worker} sample {i}"
                );
            }
        }
    }

    #[test]
    fn reset_replaces_the_snapshot_and_removes_the_file() {
        let (_tmp, store) = store();
        store
            .save_subscriptions(&[Subscription::new_from_url(
                "Example",
                "https://example.com/sub",
            )])
            .unwrap();
        assert_eq!(store.load_subscriptions().unwrap().len(), 1);

        store.reset_subscriptions().unwrap();

        assert!(store.load_subscriptions().unwrap().is_empty());
        assert!(!store.paths().subscriptions_path().exists());
    }

    #[test]
    fn reset_routing_rules_recovers_from_corrupt_file() {
        let (_tmp, store) = store();
        std::fs::write(store.paths().routing_rules_path(), b"{corrupt json").unwrap();
        assert!(store.load_routing_rules().is_err());

        store.reset_routing_rules().unwrap();

        let rules = store.load_routing_rules().unwrap();
        assert!(rules.rules().is_empty());

        let reloaded = WorkspaceStore::new(store.paths().clone())
            .load_routing_rules()
            .unwrap();
        assert!(reloaded.rules().is_empty());
    }

    #[test]
    fn reset_settings_recovers_from_a_corrupt_file() {
        let (_tmp, store) = store();
        let settings = AppSettings {
            http_port: 2091,
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        std::fs::write(store.paths().settings_path(), b"not toml [").unwrap();
        assert!(persistence::load_settings(store.paths()).is_err());

        let reset = store.reset_settings().unwrap();

        assert_eq!(*reset, AppSettings::default());
        assert_eq!(*store.settings_snapshot().unwrap(), AppSettings::default());
        assert!(!store.paths().settings_path().exists());

        let reloaded = WorkspaceStore::new(store.paths().clone())
            .load_settings()
            .unwrap();
        assert_eq!(reloaded, AppSettings::default());
    }

    #[test]
    fn reset_settings_treats_a_missing_file_like_a_present_one() {
        let (_tmp, store) = store();

        let reset = store.reset_settings().unwrap();

        assert_eq!(*reset, AppSettings::default());
        assert!(!store.paths().settings_path().exists());
    }

    #[test]
    fn reset_settings_replaces_a_stale_cache_when_the_file_is_gone() {
        let (_tmp, store) = store();
        let settings = AppSettings {
            http_port: 2091,
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        std::fs::remove_file(store.paths().settings_path()).unwrap();
        assert_eq!(store.settings_snapshot().unwrap().http_port, 2091);

        let reset = store.reset_settings().unwrap();

        assert_eq!(*reset, AppSettings::default());
        assert_eq!(*store.settings_snapshot().unwrap(), AppSettings::default());
        assert!(!store.paths().settings_path().exists());
    }
}
