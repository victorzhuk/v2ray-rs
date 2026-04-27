use std::sync::{Arc, Mutex};

use uuid::Uuid;
use v2ray_rs_core::models::{AppSettings, ManualNode, RoutingRuleSet, Subscription};
use v2ray_rs_core::persistence::{self, AppPaths, PersistenceError};
use v2ray_rs_core::resolve::LatencySnapshot;

#[derive(Clone)]
pub struct WorkspaceStore {
    paths: AppPaths,
    lock: Arc<Mutex<()>>,
}

impl WorkspaceStore {
    pub fn new(paths: AppPaths) -> Self {
        Self {
            paths,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn load_settings(&self) -> Result<AppSettings, PersistenceError> {
        self.with_lock(|| persistence::load_settings(&self.paths))
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), PersistenceError> {
        self.with_lock(|| persistence::save_settings(&self.paths, settings))
    }

    pub fn update_settings<F>(&self, mutate: F) -> Result<AppSettings, PersistenceError>
    where
        F: FnOnce(&mut AppSettings),
    {
        self.with_lock(|| {
            let mut settings = persistence::load_settings(&self.paths)?;
            mutate(&mut settings);
            persistence::save_settings(&self.paths, &settings)?;
            Ok(settings)
        })
    }

    pub fn load_subscriptions(&self) -> Result<Vec<Subscription>, PersistenceError> {
        self.with_lock(|| persistence::load_subscriptions(&self.paths))
    }

    pub fn save_subscriptions(
        &self,
        subscriptions: &[Subscription],
    ) -> Result<(), PersistenceError> {
        self.with_lock(|| persistence::save_subscriptions(&self.paths, subscriptions))
    }

    pub fn update_subscriptions<F, R>(&self, mutate: F) -> Result<R, PersistenceError>
    where
        F: FnOnce(&mut Vec<Subscription>) -> R,
    {
        self.with_lock(|| {
            let mut subscriptions = persistence::load_subscriptions(&self.paths)?;
            let result = mutate(&mut subscriptions);
            persistence::save_subscriptions(&self.paths, &subscriptions)?;
            Ok(result)
        })
    }

    pub fn get_subscription(&self, id: Uuid) -> Result<Option<Subscription>, PersistenceError> {
        self.with_lock(|| persistence::get_subscription(&self.paths, &id))
    }

    pub fn load_manual_nodes(&self) -> Result<Vec<ManualNode>, PersistenceError> {
        self.with_lock(|| persistence::load_manual_nodes(&self.paths))
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
        self.with_lock(|| persistence::save_manual_nodes(&self.paths, nodes))
    }

    pub fn update_manual_nodes<F, R>(&self, mutate: F) -> Result<R, PersistenceError>
    where
        F: FnOnce(&mut Vec<ManualNode>) -> R,
    {
        self.with_lock(|| {
            let mut nodes = persistence::load_manual_nodes(&self.paths)?;
            let result = mutate(&mut nodes);
            persistence::save_manual_nodes(&self.paths, &nodes)?;
            Ok(result)
        })
    }

    pub fn load_routing_rules(&self) -> Result<RoutingRuleSet, PersistenceError> {
        self.with_lock(|| persistence::load_routing_rules(&self.paths))
    }

    pub fn save_routing_rules(&self, rules: &RoutingRuleSet) -> Result<(), PersistenceError> {
        self.with_lock(|| persistence::save_routing_rules(&self.paths, rules))
    }

    pub fn update_routing_rules<F, R>(&self, mutate: F) -> Result<R, PersistenceError>
    where
        F: FnOnce(&mut RoutingRuleSet) -> R,
    {
        self.with_lock(|| {
            let mut rules = persistence::load_routing_rules(&self.paths)?;
            let result = mutate(&mut rules);
            persistence::save_routing_rules(&self.paths, &rules)?;
            Ok(result)
        })
    }

    pub fn load_latency_snapshot(&self) -> Result<LatencySnapshot, PersistenceError> {
        self.with_lock(|| persistence::load_latency_snapshot(&self.paths))
    }

    pub fn save_latency_snapshot(
        &self,
        snapshot: &LatencySnapshot,
    ) -> Result<(), PersistenceError> {
        self.with_lock(|| persistence::save_latency_snapshot(&self.paths, snapshot))
    }

    pub fn reset_subscriptions(&self) -> Result<(), std::io::Error> {
        self.with_std_lock(
            || match std::fs::remove_file(self.paths.subscriptions_path()) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            },
        )
    }

    pub fn reset_manual_nodes(&self) -> Result<(), std::io::Error> {
        self.with_std_lock(
            || match std::fs::remove_file(self.paths.custom_nodes_path()) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            },
        )
    }

    fn with_lock<T>(
        &self,
        f: impl FnOnce() -> Result<T, PersistenceError>,
    ) -> Result<T, PersistenceError> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }

    fn with_std_lock<T>(
        &self,
        f: impl FnOnce() -> Result<T, std::io::Error>,
    ) -> Result<T, std::io::Error> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }
}
