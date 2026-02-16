use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

use crate::models::{
    AutoResolveStrategy, LatencySample, LastSuccessMetadata, ProxyNode, Subscription,
};

#[derive(Debug, Clone)]
pub struct ConnectionCandidate {
    pub subscription_id: uuid::Uuid,
    pub subscription_name: String,
    pub node_index: usize,
    pub node: ProxyNode,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencyEntry {
    pub subscription_id: uuid::Uuid,
    pub node_index: usize,
    pub sample: LatencySample,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencySnapshot {
    pub samples: Vec<LatencyEntry>,
}

impl LatencySnapshot {
    pub fn new() -> Self {
        Self { samples: Vec::new() }
    }

    pub fn get(&self, subscription_id: uuid::Uuid, node_index: usize) -> Option<&LatencySample> {
        self.samples
            .iter()
            .find(|entry| {
                entry.subscription_id == subscription_id && entry.node_index == node_index
            })
            .map(|entry| &entry.sample)
    }

    pub fn upsert(
        &mut self,
        subscription_id: uuid::Uuid,
        node_index: usize,
        latency_ms: u64,
        measured_at: DateTime<Utc>,
    ) {
        if let Some(entry) = self.samples.iter_mut().find(|entry| {
            entry.subscription_id == subscription_id && entry.node_index == node_index
        }) {
            entry.sample = LatencySample {
                latency_ms,
                measured_at,
            };
            return;
        }
        self.samples.push(LatencyEntry {
            subscription_id,
            node_index,
            sample: LatencySample {
                latency_ms,
                measured_at,
            },
        });
    }
}

impl Default for LatencySnapshot {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ConnectionPlanner {
    strategy: AutoResolveStrategy,
    last_success: Option<LastSuccessMetadata>,
    latency_snapshot: LatencySnapshot,
    geo_preference: Vec<String>,
}

impl ConnectionPlanner {
    pub fn new(
        strategy: AutoResolveStrategy,
        last_success: Option<LastSuccessMetadata>,
        latency_snapshot: LatencySnapshot,
        geo_preference: Vec<String>,
    ) -> Self {
        Self {
            strategy,
            last_success,
            latency_snapshot,
            geo_preference,
        }
    }

    pub fn plan(&self, subscriptions: &[Subscription]) -> Vec<ConnectionCandidate> {
        let mut candidates = Vec::new();
        for sub in subscriptions.iter().filter(|s| s.enabled) {
            for (idx, node) in sub.nodes.iter().enumerate() {
                if !node.enabled {
                    continue;
                }
                let latency_ms = self
                    .latency_snapshot
                    .get(sub.id, idx)
                    .map(|sample| sample.latency_ms)
                    .or(node.last_latency_ms);
                candidates.push(ConnectionCandidate {
                    subscription_id: sub.id,
                    subscription_name: sub.name.clone(),
                    node_index: idx,
                    node: node.node.clone(),
                    latency_ms,
                });
            }
        }

        match self.strategy {
            AutoResolveStrategy::ListOrder => candidates,
            AutoResolveStrategy::LowestLatency => {
                candidates.sort_by_key(|c| c.latency_ms.unwrap_or(u64::MAX));
                candidates
            }
            AutoResolveStrategy::Random => {
                candidates.shuffle(&mut rand::rng());
                candidates
            }
            AutoResolveStrategy::LastSuccessful => {
                if let Some(last) = &self.last_success {
                    let mut prioritized = Vec::new();
                    let mut rest = Vec::new();
                    for candidate in candidates {
                        if candidate.subscription_id == last.subscription_id
                            && candidate.node_index == last.node_index
                        {
                            prioritized.push(candidate);
                        } else {
                            rest.push(candidate);
                        }
                    }
                    prioritized.extend(rest);
                    prioritized
                } else {
                    candidates
                }
            }
            AutoResolveStrategy::GeoAware => self.order_geo(candidates),
        }
    }

    fn order_geo(&self, candidates: Vec<ConnectionCandidate>) -> Vec<ConnectionCandidate> {
        if self.geo_preference.is_empty() {
            return candidates;
        }

        let mut buckets: HashMap<usize, Vec<ConnectionCandidate>> = HashMap::new();
        let mut unmatched = Vec::new();

        for candidate in candidates {
            let remark = candidate
                .node
                .remark()
                .map(|r| r.to_lowercase())
                .unwrap_or_default();

            let bucket_idx = self
                .geo_preference
                .iter()
                .enumerate()
                .find_map(|(idx, pref)| remark.contains(&pref.to_lowercase()).then_some(idx));

            if let Some(idx) = bucket_idx {
                buckets.entry(idx).or_default().push(candidate);
            } else {
                unmatched.push(candidate);
            }
        }

        let mut ordered = Vec::new();
        let mut seen = HashSet::new();
        for idx in 0..self.geo_preference.len() {
            if let Some(list) = buckets.remove(&idx) {
                for candidate in list {
                    if seen.insert((candidate.subscription_id, candidate.node_index)) {
                        ordered.push(candidate);
                    }
                }
            }
        }
        ordered.extend(unmatched);
        ordered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProxyNode, Subscription, SubscriptionNode, VlessConfig};
    use chrono::Utc;

    fn subscription_with_nodes(
        name: &str,
        nodes: Vec<(ProxyNode, bool, Option<u64>)>,
    ) -> Subscription {
        let mut sub = Subscription::new_from_url(name, "https://example.com");
        sub.nodes = nodes
            .into_iter()
            .map(|(node, enabled, latency)| SubscriptionNode {
                node,
                enabled,
                last_latency_ms: latency,
            })
            .collect();
        sub
    }

    fn vless_node(addr: &str, remark: &str) -> ProxyNode {
        ProxyNode::Vless(VlessConfig {
            address: addr.into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: crate::models::TransportSettings::Tcp,
            tls: None,
            remark: Some(remark.into()),
        })
    }

    #[test]
    fn plan_list_order_preserves_subscription_order() {
        let sub1 = subscription_with_nodes(
            "Alpha",
            vec![(vless_node("a.com", "A"), true, Some(50))],
        );
        let sub2 = subscription_with_nodes(
            "Beta",
            vec![(vless_node("b.com", "B"), true, Some(10))],
        );
        let planner = ConnectionPlanner::new(
            AutoResolveStrategy::ListOrder,
            None,
            LatencySnapshot::default(),
            Vec::new(),
        );

        let planned = planner.plan(&[sub1.clone(), sub2.clone()]);

        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].subscription_id, sub1.id);
        assert_eq!(planned[1].subscription_id, sub2.id);
    }

    #[test]
    fn plan_lowest_latency_orders_known_first() {
        let sub = subscription_with_nodes(
            "Alpha",
            vec![
                (vless_node("a.com", "A"), true, Some(120)),
                (vless_node("b.com", "B"), true, None),
                (vless_node("c.com", "C"), true, Some(40)),
            ],
        );
        let planner = ConnectionPlanner::new(
            AutoResolveStrategy::LowestLatency,
            None,
            LatencySnapshot::default(),
            Vec::new(),
        );

        let planned = planner.plan(&[sub.clone()]);

        assert_eq!(planned[0].node.address(), "c.com");
        assert_eq!(planned[1].node.address(), "a.com");
        assert_eq!(planned[2].node.address(), "b.com");
    }

    #[test]
    fn plan_last_successful_prioritizes_match() {
        let sub = subscription_with_nodes(
            "Alpha",
            vec![
                (vless_node("a.com", "A"), true, Some(10)),
                (vless_node("b.com", "B"), true, Some(20)),
            ],
        );
        let last = LastSuccessMetadata {
            subscription_id: sub.id,
            node_index: 1,
            connected_at: Utc::now(),
        };
        let planner = ConnectionPlanner::new(
            AutoResolveStrategy::LastSuccessful,
            Some(last),
            LatencySnapshot::default(),
            Vec::new(),
        );

        let planned = planner.plan(&[sub.clone()]);

        assert_eq!(planned[0].node.address(), "b.com");
    }

    #[test]
    fn plan_geo_prefers_matching_remarks() {
        let sub = subscription_with_nodes(
            "Alpha",
            vec![
                (vless_node("us.com", "US - West"), true, None),
                (vless_node("jp.com", "JP Tokyo"), true, None),
                (vless_node("de.com", "DE Berlin"), true, None),
            ],
        );
        let planner = ConnectionPlanner::new(
            AutoResolveStrategy::GeoAware,
            None,
            LatencySnapshot::default(),
            vec!["jp".into(), "us".into()],
        );

        let planned = planner.plan(&[sub.clone()]);

        assert_eq!(planned[0].node.address(), "jp.com");
        assert_eq!(planned[1].node.address(), "us.com");
    }
}
