use std::time::Duration;

use thiserror::Error;
use uuid::Uuid;
use v2ray_rs_core::models::Subscription;
use v2ray_rs_core::persistence::{self, AppPaths, PersistenceError};

use crate::fetch::FetchError;
use crate::update::{self, UpdateResult};

#[derive(Debug, Error)]
pub enum SubscriptionError {
    #[error("subscription not found: {0}")]
    NotFound(Uuid),
    #[error("fetch failed: {0}")]
    Fetch(#[from] FetchError),
    #[error("storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Clone)]
pub struct SubscriptionService {
    client: reqwest::Client,
    paths: AppPaths,
}

impl SubscriptionService {
    pub fn new(paths: AppPaths) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60))
            .user_agent("v2ray-rs/0.1")
            .build()
            .expect("failed to build HTTP client");

        Self { client, paths }
    }

    pub async fn add_and_fetch(
        &self,
        name: String,
        url: String,
    ) -> Result<Subscription, SubscriptionError> {
        let mut sub = Subscription::new_from_url(name, url);
        persistence::add_subscription(&self.paths, sub.clone())?;

        match update::update_subscription(&self.client, &mut sub).await {
            Ok(_) => {
                persistence::update_subscription(&self.paths, sub.clone())?;
            }
            Err(e) => {
                log::warn!("initial fetch failed for {}: {e}", sub.id);
            }
        }

        Ok(sub)
    }

    pub async fn refresh(
        &self,
        id: Uuid,
    ) -> Result<(Subscription, UpdateResult), SubscriptionError> {
        let mut sub = persistence::get_subscription(&self.paths, &id)?
            .ok_or(SubscriptionError::NotFound(id))?;

        let result = update::update_subscription(&self.client, &mut sub).await?;
        persistence::update_subscription(&self.paths, sub.clone())?;

        Ok((sub, result))
    }

    pub async fn refresh_all_overdue(
        &self,
        global_interval_secs: u64,
    ) -> Vec<(Uuid, Result<UpdateResult, SubscriptionError>)> {
        let subs = match persistence::load_subscriptions(&self.paths) {
            Ok(subs) => subs,
            Err(e) => {
                log::error!("failed to load subscriptions: {e}");
                return vec![];
            }
        };

        let now = chrono::Utc::now();
        let mut results = Vec::new();

        for sub in subs.iter().filter(|s| s.enabled) {
            let interval = sub
                .auto_update_interval_secs
                .unwrap_or(global_interval_secs);

            let overdue = match sub.last_updated {
                Some(last) => {
                    let elapsed = (now - last).num_seconds().max(0) as u64;
                    elapsed >= interval
                }
                None => true,
            };

            if overdue {
                let result = self.refresh(sub.id).await.map(|(_, r)| r);
                results.push((sub.id, result));
            }
        }

        results
    }
}
