use super::*;
pub(super) fn probe_targets(nodes: &[v2ray_rs_core::models::SubscriptionNode]) -> Vec<ProbeTarget> {
    nodes
        .iter()
        .map(|node| ProbeTarget {
            node_id: node.id,
            connection: node.node.clone(),
        })
        .collect()
}

pub(super) async fn run_latency_probe(
    nodes: Vec<v2ray_rs_core::models::SubscriptionNode>,
) -> (Vec<ProbeTarget>, Vec<Option<u64>>) {
    let targets = probe_targets(&nodes);
    let results = v2ray_rs_subscription::ping_nodes(&nodes).await;
    (targets, results)
}

pub(super) fn report_subscription_persist_error(
    sender: &ComponentSender<SubscriptionsPage>,
    err: &PersistenceError,
) {
    log::error!("save subscriptions: {err}");
    let _ = sender.output(SubscriptionsOutput::Notice(subscription_persist_notice(
        err,
    )));
}

/// Maps a probe result index back to the current node list through the
/// captured target. Results for nodes that no longer exist or whose
/// connection changed since the probe launched are dropped.
pub(super) fn node_by_probe_target<'a>(
    sub: &'a mut Subscription,
    targets: &[ProbeTarget],
    idx: usize,
) -> Option<&'a mut v2ray_rs_core::models::SubscriptionNode> {
    let target = targets.get(idx)?;
    sub.nodes
        .iter_mut()
        .find(|n| n.id == target.node_id && same_connection(&target.connection, &n.node))
}

pub(super) fn apply_latency_by_ids(
    sub: &mut Subscription,
    targets: &[ProbeTarget],
    results: &[Option<u64>],
) {
    for (idx, latency) in results.iter().enumerate() {
        if let Some(node) = node_by_probe_target(sub, targets, idx) {
            node.last_latency_ms = *latency;
        }
    }
}

pub(super) fn apply_real_delay_by_ids(
    sub: &mut Subscription,
    targets: &[ProbeTarget],
    results: &[Option<u64>],
) {
    for (idx, delay) in results.iter().enumerate() {
        if let Some(node) = node_by_probe_target(sub, targets, idx) {
            node.last_real_delay_ms = *delay;
        }
    }
}

/// Enqueues a load-modify-save of the latency snapshot on the shared writer.
pub(super) fn persist_latency_snapshot<F>(
    sender: &ComponentSender<SubscriptionsPage>,
    store: &WorkspaceStore,
    apply: F,
) where
    F: FnOnce(&mut v2ray_rs_core::resolve::LatencySnapshot) + Send + 'static,
{
    let receipt = store.enqueue_update_latency_snapshot(move |snapshot| {
        apply(snapshot);
    });
    sender.oneshot_command(async move {
        let result = match receipt.await {
            Ok(result) => result.map(|_| ()),
            Err(_) => Err(PersistenceError::CorruptConfig(writer_stopped_message())),
        };
        SubscriptionsCmdOutput::SnapshotSaved(result)
    });
}

/// Drops in-memory egress and verdict state for nodes whose connection changed
/// in a refresh and queues a snapshot rewrite without their samples.
pub(super) fn invalidate_node_samples(
    sender: &ComponentSender<SubscriptionsPage>,
    store: &WorkspaceStore,
    egress_samples: &mut HashMap<(Uuid, Uuid), EgressSample>,
    verdicts: &mut HashMap<(Uuid, Uuid), HashMap<String, VerdictRecord>>,
    subscription_id: Uuid,
    node_ids: &[Uuid],
) {
    trim_node_samples(egress_samples, verdicts, subscription_id, node_ids);
    let node_ids = node_ids.to_vec();
    persist_latency_snapshot(sender, store, move |snapshot| {
        remove_connection_samples(snapshot, subscription_id, &node_ids);
    });
}

/// Drops in-memory egress and verdict state for every refreshed subscription's
/// changed nodes and queues one snapshot rewrite without all their samples.
pub(super) fn invalidate_node_samples_batch(
    sender: &ComponentSender<SubscriptionsPage>,
    store: &WorkspaceStore,
    egress_samples: &mut HashMap<(Uuid, Uuid), EgressSample>,
    verdicts: &mut HashMap<(Uuid, Uuid), HashMap<String, VerdictRecord>>,
    changed: &[(Uuid, Vec<Uuid>)],
) {
    for (subscription_id, node_ids) in changed {
        trim_node_samples(egress_samples, verdicts, *subscription_id, node_ids);
    }
    if changed.is_empty() {
        return;
    }
    let changed = changed.to_vec();
    persist_latency_snapshot(sender, store, move |snapshot| {
        for (subscription_id, node_ids) in &changed {
            remove_connection_samples(snapshot, *subscription_id, node_ids);
        }
    });
}

fn trim_node_samples(
    egress_samples: &mut HashMap<(Uuid, Uuid), EgressSample>,
    verdicts: &mut HashMap<(Uuid, Uuid), HashMap<String, VerdictRecord>>,
    subscription_id: Uuid,
    node_ids: &[Uuid],
) {
    let changed: HashSet<Uuid> = node_ids.iter().copied().collect();
    egress_samples
        .retain(|(sub_id, node_id), _| *sub_id != subscription_id || !changed.contains(node_id));
    verdicts
        .retain(|(sub_id, node_id), _| *sub_id != subscription_id || !changed.contains(node_id));
}

fn remove_connection_samples(
    snapshot: &mut v2ray_rs_core::resolve::LatencySnapshot,
    subscription_id: Uuid,
    node_ids: &[Uuid],
) {
    for node_id in node_ids {
        snapshot.remove(ConnectionNodeRef::Subscription {
            subscription_id,
            node_id: *node_id,
        });
    }
}

pub(super) fn subscriptions_eligible_for_latency_test(
    subscriptions: &[Subscription],
    testing_latency: &HashSet<Uuid>,
    tun_active: bool,
) -> Vec<Uuid> {
    if tun_active {
        return Vec::new();
    }
    subscriptions
        .iter()
        .filter(|sub| sub.enabled && sub.has_enabled_nodes() && !testing_latency.contains(&sub.id))
        .map(|sub| sub.id)
        .collect()
}
