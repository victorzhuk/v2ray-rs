use std::time::Duration;

use v2ray_rs_core::models::{AppSettings, BackendType};
use v2ray_rs_core::persistence::AppPaths;

#[derive(Clone)]
pub struct GeodataRefreshConfig {
    pub paths: AppPaths,
    pub backend_type: BackendType,
    pub enabled: bool,
    pub interval_secs: u64,
}

impl GeodataRefreshConfig {
    pub fn from_settings(paths: &AppPaths, settings: &AppSettings) -> Self {
        Self {
            paths: paths.clone(),
            backend_type: settings.backend.backend_type,
            enabled: settings.auto_update_geodata && settings.backend.binary_path.is_some(),
            interval_secs: settings.geodata_update_interval_secs.max(60),
        }
    }
}

#[derive(Clone)]
pub struct GeodataRefreshService {
    config_tx: tokio::sync::watch::Sender<GeodataRefreshConfig>,
}

impl GeodataRefreshService {
    pub fn spawn(initial: GeodataRefreshConfig) -> Self {
        let (config_tx, config_rx) = tokio::sync::watch::channel(initial);
        tokio::spawn(async move {
            run_loop(config_rx).await;
        });
        Self { config_tx }
    }

    pub fn update(&self, config: GeodataRefreshConfig) {
        let _ = self.config_tx.send(config);
    }
}

async fn run_loop(mut config_rx: tokio::sync::watch::Receiver<GeodataRefreshConfig>) {
    loop {
        let config = config_rx.borrow().clone();
        if !config.enabled {
            if config_rx.changed().await.is_err() {
                break;
            }
            continue;
        }

        let refresh_config = config.clone();
        let result = tokio::task::spawn_blocking(move || refresh_once(refresh_config)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => log::warn!("geodata refresh failed: {err}"),
            Err(err) => log::warn!("geodata refresh task failed: {err}"),
        }

        let sleep = tokio::time::sleep(Duration::from_secs(config.interval_secs));
        tokio::pin!(sleep);

        tokio::select! {
            _ = &mut sleep => {}
            changed = config_rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }
}

fn refresh_once(config: GeodataRefreshConfig) -> Result<(), String> {
    #[cfg(feature = "geodata-fetch")]
    {
        use v2ray_rs_core::geodata::{GeodataManager, check_and_download};
        use v2ray_rs_core::geodata_index::GeodataIndexManager;

        let manager = GeodataManager::new(&config.paths);
        let interval = Duration::from_secs(config.interval_secs);
        let changed = check_and_download(&manager, config.backend_type, interval)
            .map_err(|err| err.to_string())?;

        let index_missing = GeodataIndexManager::new(&config.paths)
            .load_index(config.backend_type)
            .map_err(|err| err.to_string())?
            .is_none();

        if changed.is_some() || index_missing {
            manager
                .reindex(config.backend_type)
                .map_err(|err| err.to_string())?;
        }

        Ok(())
    }

    #[cfg(not(feature = "geodata-fetch"))]
    {
        let _ = config;
        Ok(())
    }
}
