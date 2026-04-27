use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

use crate::models::{
    AutoResolveStrategy, ConnectionNodeRef, LastSuccessMetadata, LatencySample, ManualNode,
    ProxyNode, Subscription,
};

#[derive(Debug, Clone)]
pub struct ConnectionCandidate {
    pub node_ref: ConnectionNodeRef,
    pub source_name: String,
    pub node: ProxyNode,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LatencyEntryWire {
    node_ref: ConnectionNodeRef,
    sample: LatencySample,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LatencySnapshotWire {
    samples: Vec<LatencyEntryWire>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LatencySnapshot {
    samples: HashMap<ConnectionNodeRef, LatencySample>,
}

impl Serialize for LatencySnapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let entries: Vec<LatencyEntryWire> = self
            .samples
            .iter()
            .map(|(node_ref, sample)| LatencyEntryWire {
                node_ref: *node_ref,
                sample: sample.clone(),
            })
            .collect();
        let wire = LatencySnapshotWire { samples: entries };
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LatencySnapshot {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = LatencySnapshotWire::deserialize(deserializer)?;
        let samples = wire
            .samples
            .into_iter()
            .map(|entry| (entry.node_ref, entry.sample))
            .collect();
        Ok(LatencySnapshot { samples })
    }
}

impl LatencySnapshot {
    pub fn new() -> Self {
        Self {
            samples: HashMap::new(),
        }
    }

    pub fn get(&self, node_ref: ConnectionNodeRef) -> Option<&LatencySample> {
        self.samples.get(&node_ref)
    }

    pub fn upsert(
        &mut self,
        node_ref: ConnectionNodeRef,
        latency_ms: u64,
        measured_at: DateTime<Utc>,
    ) {
        self.samples.insert(
            node_ref,
            LatencySample {
                latency_ms,
                measured_at,
            },
        );
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
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
}

impl ConnectionPlanner {
    pub fn new(
        strategy: AutoResolveStrategy,
        last_success: Option<LastSuccessMetadata>,
        latency_snapshot: LatencySnapshot,
    ) -> Self {
        Self {
            strategy,
            last_success,
            latency_snapshot,
        }
    }

    pub fn plan(
        &self,
        subscriptions: &[Subscription],
        manual_nodes: &[ManualNode],
    ) -> Vec<ConnectionCandidate> {
        let mut candidates = Vec::new();

        for sub in subscriptions.iter().filter(|s| s.enabled) {
            for node in &sub.nodes {
                if !node.enabled {
                    continue;
                }
                let node_ref = ConnectionNodeRef::Subscription {
                    subscription_id: sub.id,
                    node_id: node.id,
                };
                let latency_ms = self
                    .latency_snapshot
                    .get(node_ref)
                    .map(|sample| sample.latency_ms)
                    .or(node.last_latency_ms);
                candidates.push(ConnectionCandidate {
                    node_ref,
                    source_name: sub.name.clone(),
                    node: node.node.clone(),
                    latency_ms,
                });
            }
        }

        for manual in manual_nodes.iter().filter(|n| n.enabled) {
            let node_ref = ConnectionNodeRef::Manual { node_id: manual.id };
            let latency_ms = self
                .latency_snapshot
                .get(node_ref)
                .map(|sample| sample.latency_ms);
            candidates.push(ConnectionCandidate {
                node_ref,
                source_name: "Manual".to_string(),
                node: manual.node.clone(),
                latency_ms,
            });
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
                        if candidate.node_ref == last.node_ref {
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
        }
    }

    pub fn runtime_candidate(
        &self,
        subscriptions: &[Subscription],
        manual_nodes: &[ManualNode],
    ) -> Option<ConnectionCandidate> {
        match self.strategy {
            // Keep disconnected config regeneration deterministic.
            AutoResolveStrategy::Random => ConnectionPlanner::new(
                AutoResolveStrategy::ListOrder,
                self.last_success.clone(),
                self.latency_snapshot.clone(),
            )
            .plan(subscriptions, manual_nodes)
            .into_iter()
            .next(),
            _ => self.plan(subscriptions, manual_nodes).into_iter().next(),
        }
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
            .map(|(node, enabled, latency)| {
                let mut subscription_node = SubscriptionNode::new(node);
                subscription_node.enabled = enabled;
                subscription_node.last_latency_ms = latency;
                subscription_node
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
        let sub1 =
            subscription_with_nodes("Alpha", vec![(vless_node("a.com", "A"), true, Some(50))]);
        let sub2 =
            subscription_with_nodes("Beta", vec![(vless_node("b.com", "B"), true, Some(10))]);
        let planner = ConnectionPlanner::new(
            AutoResolveStrategy::ListOrder,
            None,
            LatencySnapshot::default(),
        );

        let planned = planner.plan(&[sub1.clone(), sub2.clone()], &[]);

        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].source_name, "Alpha");
        assert_eq!(planned[1].source_name, "Beta");
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
        );

        let planned = planner.plan(std::slice::from_ref(&sub), &[]);

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
        let target_node_id = sub.nodes[1].id;
        let last = LastSuccessMetadata {
            node_ref: ConnectionNodeRef::Subscription {
                subscription_id: sub.id,
                node_id: target_node_id,
            },
            connected_at: Utc::now(),
        };
        let planner = ConnectionPlanner::new(
            AutoResolveStrategy::LastSuccessful,
            Some(last),
            LatencySnapshot::default(),
        );

        let planned = planner.plan(std::slice::from_ref(&sub), &[]);

        assert_eq!(planned[0].node.address(), "b.com");
    }

    #[test]
    fn runtime_candidate_uses_stable_order_for_random_strategy() {
        let sub1 =
            subscription_with_nodes("Alpha", vec![(vless_node("a.com", "A"), true, Some(50))]);
        let sub2 =
            subscription_with_nodes("Beta", vec![(vless_node("b.com", "B"), true, Some(10))]);
        let planner = ConnectionPlanner::new(
            AutoResolveStrategy::Random,
            None,
            LatencySnapshot::default(),
        );

        let candidate = planner.runtime_candidate(&[sub1, sub2], &[]).unwrap();

        assert_eq!(candidate.source_name, "Alpha");
        assert_eq!(candidate.node.address(), "a.com");
    }

    #[test]
    fn test_latency_stable_after_manual_node_insert() {
        let snapshot = LatencySnapshot::new();
        let node_id = uuid::Uuid::new_v4();

        let node_ref = ConnectionNodeRef::Manual { node_id };
        let now = Utc::now();

        let mut snapshot = snapshot;
        snapshot.upsert(node_ref, 100, now);

        snapshot.upsert(
            ConnectionNodeRef::Manual {
                node_id: uuid::Uuid::new_v4(),
            },
            200,
            now,
        );

        let latency = snapshot.get(node_ref);
        assert!(latency.is_some());
        assert_eq!(latency.unwrap().latency_ms, 100);
    }

    #[test]
    fn test_last_success_ref_stable_across_operations() {
        let node_id = uuid::Uuid::new_v4();
        let last = LastSuccessMetadata {
            node_ref: ConnectionNodeRef::Manual { node_id },
            connected_at: Utc::now(),
        };

        let json = serde_json::to_string(&last).unwrap();
        let loaded: LastSuccessMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.node_ref, last.node_ref);
    }
}
