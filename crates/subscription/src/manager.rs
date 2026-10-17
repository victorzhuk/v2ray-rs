use std::sync::Arc;

use futures_util::{StreamExt, stream};
use thiserror::Error;
use tokio::sync::Semaphore;
use uuid::Uuid;
use v2ray_rs_core::models::{Subscription, SubscriptionSource};

use crate::fetch::{CONNECT_TIMEOUT, REDIRECT_CAP, REQUEST_TIMEOUT, USER_AGENT, redirect_policy};
use crate::update::{self, UpdateError, UpdateResult};

const MAX_CONCURRENT_REFRESHES: usize = 4;

#[derive(Debug, Clone)]
pub struct SubscriptionImportOutcome {
    pub subscription: Subscription,
    pub result: UpdateResult,
}

#[derive(Debug, Error)]
pub enum SubscriptionError {
    #[error("subscription client initialization failed: {0}")]
    ClientInit(String),
    #[error("update failed: {0}")]
    Update(#[from] UpdateError),
}

#[derive(Clone)]
pub struct SubscriptionService {
    client: Option<reqwest::Client>,
    client_error: Option<String>,
    /// Shared admission for every refresh entry point. Clones of the service
    /// hold the same permits, so overlapping `refresh`, `add_and_fetch`, and
    /// `refresh_all_overdue` calls never exceed `MAX_CONCURRENT_REFRESHES`
    /// concurrent updates in total.
    refresh_gate: Arc<Semaphore>,
}

impl Default for SubscriptionService {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriptionService {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(USER_AGENT)
            .redirect(redirect_policy(REDIRECT_CAP))
            .build();
        let refresh_gate = Arc::new(Semaphore::new(MAX_CONCURRENT_REFRESHES));

        match client {
            Ok(client) => Self {
                client: Some(client),
                client_error: None,
                refresh_gate,
            },
            Err(err) => {
                log::error!("subscription HTTP client init: {err}");
                Self {
                    client: None,
                    client_error: Some(err.to_string()),
                    refresh_gate,
                }
            }
        }
    }

    fn client(&self) -> Result<&reqwest::Client, SubscriptionError> {
        self.client.as_ref().ok_or_else(|| {
            SubscriptionError::ClientInit(
                self.client_error
                    .clone()
                    .unwrap_or_else(|| "HTTP client unavailable".into()),
            )
        })
    }

    pub async fn add_and_fetch(
        &self,
        name: String,
        source: SubscriptionSource,
    ) -> Result<SubscriptionImportOutcome, SubscriptionError> {
        let _permit = self.refresh_gate.acquire().await.ok();
        let client = self.client()?;
        let mut sub = match source {
            SubscriptionSource::Url { url } => Subscription::new_from_url(name, url),
            SubscriptionSource::File { path } => Subscription::new_from_file(name, path),
        };
        let result = update::update_subscription(client, &mut sub).await?;

        Ok(SubscriptionImportOutcome {
            subscription: sub,
            result,
        })
    }

    pub async fn refresh(
        &self,
        mut sub: Subscription,
    ) -> Result<(Subscription, UpdateResult), SubscriptionError> {
        let _permit = self.refresh_gate.acquire().await.ok();
        let client = self.client()?;
        let result = update::update_subscription(client, &mut sub).await?;

        Ok((sub, result))
    }

    pub async fn refresh_all_overdue(
        &self,
        subscriptions: Vec<Subscription>,
        global_interval_secs: u64,
    ) -> Vec<(
        Uuid,
        Result<(Subscription, UpdateResult), SubscriptionError>,
    )> {
        let now = chrono::Utc::now();
        let overdue: Vec<_> = subscriptions
            .into_iter()
            .filter(|s| s.enabled)
            .filter(|sub| {
                let interval = sub
                    .auto_update_interval_secs
                    .unwrap_or(global_interval_secs);

                match sub.last_updated {
                    Some(last) => {
                        let elapsed = (now - last).num_seconds().max(0) as u64;
                        elapsed >= interval
                    }
                    None => true,
                }
            })
            .collect();

        if overdue.is_empty() {
            return Vec::new();
        }

        if let Some(err) = &self.client_error {
            return overdue
                .into_iter()
                .map(|sub| (sub.id, Err(SubscriptionError::ClientInit(err.clone()))))
                .collect();
        }

        stream::iter(overdue.into_iter().map(|sub| {
            let svc = self.clone();
            async move {
                let id = sub.id;
                let result = svc.refresh(sub).await;
                (id, result)
            }
        }))
        .buffer_unordered(MAX_CONCURRENT_REFRESHES)
        .collect()
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_refresh_admission() {
        let service = SubscriptionService::new();
        let clone = service.clone();
        assert!(Arc::ptr_eq(&service.refresh_gate, &clone.refresh_gate));
        assert_eq!(
            service.refresh_gate.available_permits(),
            MAX_CONCURRENT_REFRESHES
        );
    }
}
