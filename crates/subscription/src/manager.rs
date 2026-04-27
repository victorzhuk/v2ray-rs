use futures_util::{StreamExt, stream};
use thiserror::Error;
use uuid::Uuid;
use v2ray_rs_core::models::{Subscription, SubscriptionSource};

use crate::fetch::{CONNECT_TIMEOUT, REQUEST_TIMEOUT, USER_AGENT};
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
            .build();

        match client {
            Ok(client) => Self {
                client: Some(client),
                client_error: None,
            },
            Err(err) => {
                log::error!("subscription HTTP client init: {err}");
                Self {
                    client: None,
                    client_error: Some(err.to_string()),
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
