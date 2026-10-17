use adw::prelude::*;
use base64::Engine;
use gtk::gdk;
use relm4::adw;
use relm4::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use uuid::Uuid;

use v2ray_rs_core::geo_probe::{AvailabilityVerdict, ServiceCheck, builtin_service_checks};
use v2ray_rs_core::models::{
    AppSettings, BackendType, ConnectionNodeRef, EgressSample, RealDelayCapability,
    RealDelaySettings, Subscription, SubscriptionSource, VerdictRecord,
};
use v2ray_rs_core::persistence::{AppPaths, PersistenceError};
use v2ray_rs_core::runtime_snapshot::subscriptions_runtime_state_eq;
use v2ray_rs_subscription::{
    RealDelayReport, SubscriptionError, SubscriptionImportOutcome, SubscriptionService,
    UpdateError, UpdateResult, reconcile_nodes,
};

use crate::failure_streak::FailureStreak;
use crate::workspace::WorkspaceStore;

pub struct SubscriptionsPage {
    store: WorkspaceStore,
    service: SubscriptionService,
    subscriptions: Vec<Subscription>,
    list_container: gtk::ListBox,
    load_error: Option<String>,
    auto_update_enabled: bool,
    auto_update_interval_secs: u64,
    auto_update_generation: u64,
    auto_update_streaks: HashMap<Uuid, FailureStreak>,
    testing_latency: HashSet<Uuid>,
    refreshing: HashSet<Uuid>,
    persister: SubscriptionPersister,
    testing_real_delay: HashMap<Uuid, u64>,
    real_delay_run_token: u64,
    testing_availability: HashMap<Uuid, u64>,
    availability_run_token: u64,
    egress_samples: HashMap<(Uuid, Uuid), EgressSample>,
    verdicts: HashMap<(Uuid, Uuid), HashMap<String, VerdictRecord>>,
    expanded_subs: HashSet<Uuid>,
    active_node: Option<(Uuid, Uuid)>,
    tun_active: bool,
    backend_type: BackendType,
    binary_path: Option<std::path::PathBuf>,
    real_delay_settings: RealDelaySettings,
    real_delay_capability: RealDelayCapability,
    paths: AppPaths,
}

enum RenderHint {
    Full,
    NodeToggle(Uuid, usize),
    SubscriptionToggle(Uuid),
    SubscriptionRename(Uuid),
}

struct RenderState<'a> {
    expanded_subs: &'a HashSet<Uuid>,
    testing_latency: &'a HashSet<Uuid>,
    testing_real_delay: &'a HashMap<Uuid, u64>,
    real_delay_available: bool,
    real_delay_capability: &'a RealDelayCapability,
    locked: bool,
    active_node: Option<(Uuid, Uuid)>,
    tun_active: bool,
    egress_samples: &'a HashMap<(Uuid, Uuid), EgressSample>,
    verdicts: &'a HashMap<(Uuid, Uuid), HashMap<String, VerdictRecord>>,
    testing_availability: &'a HashMap<Uuid, u64>,
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
}

#[derive(Debug)]
pub enum SubscriptionsOutput {
    ActiveNodesChanged(bool),
    SubscriptionsChanged,
    ConnectNode(Uuid, Uuid),
    Notice(String),
}

type AutoUpdateRefreshResult = Result<(Subscription, UpdateResult), String>;
type AutoUpdateRefreshBatch = Vec<(Uuid, AutoUpdateRefreshResult)>;

#[derive(Debug)]
pub enum SubscriptionsMsg {
    ToggleSubscription(Uuid),
    ToggleUseImportedProfile(Uuid),
    ToggleNode(Uuid, usize),
    DeleteSubscription(Uuid),
    RenameSubscription(Uuid, String),
    MoveSubscription(Uuid, Direction),
    MoveNode(Uuid, usize, Direction),
    AddSubscription(String, SubscriptionSourceInput),
    UpdateSubscription(Uuid),
    TestLatency(Uuid),
    SortByLatency(Uuid),
    TestRealDelay(Uuid),
    SortByRealDelay(Uuid),
    CheckAvailability(Uuid),
    ShowAvailability(Uuid),
    EnableAllNodes(Uuid),
    DisableAllNodes(Uuid),
    DragDropSubscription(usize, usize),
    DragDropNode(Uuid, usize, usize),
    ConnectNode(Uuid, Uuid),
    CheckAutoUpdate,
    SyncSettings {
        auto_update_enabled: bool,
        auto_update_interval_secs: u64,
        backend_type: BackendType,
        binary_path: Option<std::path::PathBuf>,
        real_delay_settings: RealDelaySettings,
    },
    ResetStorage,
    SetActiveNode(Option<(Uuid, Uuid)>),
    SetTunActive(bool),
    ExpanderToggled(Uuid, bool),
}

#[derive(Debug)]
pub enum SubscriptionsCmdOutput {
    AddDone(SubscriptionImportOutcome),
    RefreshDone(Subscription, UpdateResult),
    LatencyResult(Uuid, Vec<Uuid>, Vec<Option<u64>>),
    RealDelayResult(Uuid, u64, Vec<Uuid>, RealDelayReport),
    AvailabilityResult(Uuid, u64, Vec<Uuid>, RealDelayReport),
    RefreshFailed(Uuid, SubscriptionError),
    AddFailed(SubscriptionError),
    AutoUpdateDone(AutoUpdateRefreshBatch),
    AutoUpdateTick(u64),
    BackgroundLatencyTick,
    SubscriptionsSaved(Result<(), PersistenceError>),
    SnapshotSaved(Result<(), PersistenceError>),
}

/// Serializes full-snapshot saves off the GTK thread. Latest snapshot wins:
/// a job that loses the race against a newer dispatch writes the newer
/// snapshot and skips its own stale one.
#[derive(Clone, Default)]
struct SubscriptionPersister {
    pending: Arc<Mutex<Option<Vec<Subscription>>>>,
}

impl SubscriptionPersister {
    fn submit(
        &self,
        store: &WorkspaceStore,
        sender: &ComponentSender<SubscriptionsPage>,
        snapshot: Vec<Subscription>,
    ) {
        *self.lock_pending() = Some(snapshot);
        let this = self.clone();
        let store = store.clone();
        sender.oneshot_command(async move {
            let result = tokio::task::spawn_blocking(move || this.drain(&store))
                .await
                .unwrap_or_else(|join_err| {
                    Err(PersistenceError::Io(std::io::Error::other(join_err)))
                });
            SubscriptionsCmdOutput::SubscriptionsSaved(result)
        });
    }

    fn lock_pending(&self) -> MutexGuard<'_, Option<Vec<Subscription>>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn drain(&self, store: &WorkspaceStore) -> Result<(), PersistenceError> {
        while let Some(snapshot) = self.lock_pending().take() {
            store.save_subscriptions(&snapshot)?;
        }
        Ok(())
    }
}

fn commit_subscriptions_mutation<R>(
    subscriptions: &mut Vec<Subscription>,
    mutate: impl FnOnce(&mut Vec<Subscription>) -> Option<R>,
) -> Option<R> {
    let mut next = subscriptions.clone();
    let result = mutate(&mut next)?;
    *subscriptions = next;
    Some(result)
}

fn subscription_persist_notice(err: &PersistenceError) -> String {
    format!("Failed to save subscriptions: {err}")
}

fn should_start_refresh(refreshing: &HashSet<Uuid>, id: Uuid) -> bool {
    !refreshing.contains(&id)
}

async fn run_latency_probe(
    nodes: Vec<v2ray_rs_core::models::SubscriptionNode>,
) -> (Vec<Uuid>, Vec<Option<u64>>) {
    let mut snapshot_ids = Vec::with_capacity(nodes.len());
    for node in &nodes {
        snapshot_ids.push(node.id);
    }
    let results = v2ray_rs_subscription::ping_nodes(&nodes).await;
    (snapshot_ids, results)
}

fn report_subscription_persist_error(
    sender: &ComponentSender<SubscriptionsPage>,
    err: &PersistenceError,
) {
    log::error!("save subscriptions: {err}");
    let _ = sender.output(SubscriptionsOutput::Notice(subscription_persist_notice(
        err,
    )));
}

/// Maps a probe result index back to the current node list through the
/// snapshot's node ids. Results for nodes that no longer exist are dropped.
fn node_by_snapshot_id<'a>(
    sub: &'a mut Subscription,
    snapshot_ids: &[Uuid],
    idx: usize,
) -> Option<&'a mut v2ray_rs_core::models::SubscriptionNode> {
    let id = *snapshot_ids.get(idx)?;
    sub.nodes.iter_mut().find(|n| n.id == id)
}

fn apply_latency_by_ids(sub: &mut Subscription, snapshot_ids: &[Uuid], results: &[Option<u64>]) {
    for (idx, latency) in results.iter().enumerate() {
        if let Some(node) = node_by_snapshot_id(sub, snapshot_ids, idx) {
            node.last_latency_ms = *latency;
        }
    }
}

fn apply_real_delay_by_ids(sub: &mut Subscription, snapshot_ids: &[Uuid], results: &[Option<u64>]) {
    for (idx, delay) in results.iter().enumerate() {
        if let Some(node) = node_by_snapshot_id(sub, snapshot_ids, idx) {
            node.last_real_delay_ms = *delay;
        }
    }
}

/// Runs a load-modify-save of the latency snapshot on a blocking thread.
fn persist_latency_snapshot<F>(
    sender: &ComponentSender<SubscriptionsPage>,
    store: &WorkspaceStore,
    apply: F,
) where
    F: FnOnce(&mut v2ray_rs_core::resolve::LatencySnapshot) + Send + 'static,
{
    let store = store.clone();
    sender.oneshot_command(async move {
        let result = tokio::task::spawn_blocking(move || {
            let mut snapshot = store.load_latency_snapshot()?;
            apply(&mut snapshot);
            store.save_latency_snapshot(&snapshot)
        })
        .await
        .unwrap_or_else(|join_err| Err(PersistenceError::Io(std::io::Error::other(join_err))));
        SubscriptionsCmdOutput::SnapshotSaved(result)
    });
}

#[relm4::component(pub)]
impl Component for SubscriptionsPage {
    type Init = (WorkspaceStore, AppSettings);
    type Input = SubscriptionsMsg;
    type Output = SubscriptionsOutput;
    type CommandOutput = SubscriptionsCmdOutput;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 0,

            gtk::Revealer {
                #[watch]
                set_reveal_child: model.load_error.is_some(),

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    set_margin_top: 6,
                    set_margin_bottom: 6,
                    set_margin_start: 12,
                    set_margin_end: 12,

                    gtk::Label {
                        set_xalign: 0.0,
                        set_hexpand: true,
                        add_css_class: "warning",
                        #[watch]
                        set_label: model
                            .load_error
                            .as_deref()
                            .unwrap_or("Failed to load subscriptions"),
                    },

                    gtk::Button {
                        set_label: "Reset Data",
                        add_css_class: "destructive-action",
                        connect_clicked => SubscriptionsMsg::ResetStorage,
                    },
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::End,
                set_margin_top: 6,
                set_margin_end: 6,

                gtk::Button {
                    set_icon_name: "list-add-symbolic",
                    set_tooltip_text: Some("Add Subscription"),
                    add_css_class: "flat",
                    #[watch]
                    set_sensitive: model.load_error.is_none(),
                    connect_clicked[sender] => move |_| {
                        show_add_dialog(sender.clone());
                    },
                },
            },

            gtk::ScrolledWindow {
                set_vexpand: true,
                set_hexpand: true,

                #[wrap(Some)]
                set_child = &model.list_container.clone(),
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (store, settings) = init;
        let service = SubscriptionService::new();
        let (mut subscriptions, load_error) = match store.load_subscriptions() {
            Ok(subscriptions) => (subscriptions, None),
            Err(err) => (
                Vec::new(),
                Some(format!("Subscriptions are read-only: {err}")),
            ),
        };
        let mut egress_samples = HashMap::new();
        let mut verdicts = HashMap::new();
        if let Ok(snapshot) = store.load_latency_snapshot() {
            for sub in &mut subscriptions {
                for node in &mut sub.nodes {
                    let node_ref = v2ray_rs_core::models::ConnectionNodeRef::Subscription {
                        subscription_id: sub.id,
                        node_id: node.id,
                    };
                    if let Some(sample) = snapshot.get(node_ref) {
                        node.last_latency_ms = sample.latency_ms;
                        node.last_real_delay_ms = sample.real_delay_ms;
                        if let Some(egress) = sample.egress.clone() {
                            egress_samples.insert((sub.id, node.id), egress);
                        }
                        if !sample.verdicts.is_empty() {
                            verdicts.insert((sub.id, node.id), sample.verdicts.clone());
                        }
                    }
                }
            }
        }

        let list_container = gtk::ListBox::builder()
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["boxed-list"])
            .selection_mode(gtk::SelectionMode::None)
            .build();

        let model = SubscriptionsPage {
            store: store.clone(),
            service,
            subscriptions,
            list_container: list_container.clone(),
            load_error,
            auto_update_enabled: settings.auto_update_subscriptions,
            auto_update_interval_secs: settings.subscription_update_interval_secs,
            auto_update_generation: 0,
            auto_update_streaks: HashMap::new(),
            testing_latency: HashSet::new(),
            refreshing: HashSet::new(),
            persister: SubscriptionPersister::default(),
            testing_real_delay: HashMap::new(),
            real_delay_run_token: 0,
            testing_availability: HashMap::new(),
            availability_run_token: 0,
            egress_samples,
            verdicts,
            expanded_subs: HashSet::new(),
            active_node: None,
            tun_active: false,
            backend_type: settings.backend.backend_type,
            binary_path: settings.backend.binary_path.clone(),
            real_delay_settings: settings.real_delay.clone(),
            real_delay_capability: settings
                .backend
                .backend_type
                .default_real_delay_capability(),
            paths: store.paths().clone(),
        };

        render_list(
            &model.subscriptions,
            &list_container,
            &sender,
            RenderState {
                expanded_subs: &model.expanded_subs,
                testing_latency: &model.testing_latency,
                testing_real_delay: &model.testing_real_delay,
                real_delay_available: model.real_delay_settings.enabled
                    && model.backend_type.supports_real_delay(),
                real_delay_capability: &model.real_delay_capability,
                locked: model.load_error.is_some(),
                active_node: model.active_node,
                tun_active: model.tun_active,
                egress_samples: &model.egress_samples,
                verdicts: &model.verdicts,
                testing_availability: &model.testing_availability,
            },
        );

        if settings.auto_update_subscriptions {
            sender.input(SubscriptionsMsg::CheckAutoUpdate);
            schedule_auto_update_tick(
                sender.clone(),
                0,
                settings.subscription_update_interval_secs,
            );
        }

        sender.oneshot_command(async move {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            SubscriptionsCmdOutput::BackgroundLatencyTick
        });

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        let emit_active_nodes = |subs: &[Subscription], sender: &ComponentSender<Self>| {
            let has_active = subs.iter().any(|s| s.has_enabled_nodes());
            let _ = sender.output(SubscriptionsOutput::ActiveNodesChanged(has_active));
        };

        if self.load_error.is_some()
            && !matches!(
                msg,
                SubscriptionsMsg::ResetStorage
                    | SubscriptionsMsg::SetActiveNode(_)
                    | SubscriptionsMsg::SetTunActive(_)
                    | SubscriptionsMsg::SyncSettings { .. }
            )
        {
            return;
        }

        let mut subscriptions_changed = false;
        let mut render_hint = RenderHint::Full;

        match msg {
            SubscriptionsMsg::ToggleSubscription(id) => {
                if let Some(hint) =
                    commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                        sub.enabled = !sub.enabled;
                        Some(RenderHint::SubscriptionToggle(id))
                    })
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                    render_hint = hint;
                }
            }
            SubscriptionsMsg::ToggleUseImportedProfile(id) => {
                if commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                    let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                    sub.use_imported_profile = !sub.use_imported_profile;
                    Some(())
                })
                .is_some()
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                }
            }
            SubscriptionsMsg::ToggleNode(sub_id, idx) => {
                if let Some(hint) =
                    commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == sub_id)?;
                        let node = sub.nodes.get_mut(idx)?;
                        node.enabled = !node.enabled;
                        Some(RenderHint::NodeToggle(sub_id, idx))
                    })
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                    render_hint = hint;
                }
            }
            SubscriptionsMsg::RenameSubscription(id, new_name) => {
                if let Some(hint) =
                    commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                        sub.name = new_name;
                        Some(RenderHint::SubscriptionRename(id))
                    })
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    render_hint = hint;
                }
            }
            SubscriptionsMsg::MoveSubscription(id, direction) => {
                if commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                    let pos = subscriptions.iter().position(|s| s.id == id)?;
                    let new_pos = match direction {
                        Direction::Up if pos > 0 => pos - 1,
                        Direction::Down if pos + 1 < subscriptions.len() => pos + 1,
                        _ => pos,
                    };
                    if new_pos == pos {
                        return None;
                    }
                    subscriptions.swap(pos, new_pos);
                    Some(())
                })
                .is_some()
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                }
            }
            SubscriptionsMsg::MoveNode(sub_id, idx, direction) => {
                if commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                    let sub = subscriptions.iter_mut().find(|s| s.id == sub_id)?;
                    let new_idx = match direction {
                        Direction::Up if idx > 0 => idx - 1,
                        Direction::Down if idx + 1 < sub.nodes.len() => idx + 1,
                        _ => idx,
                    };
                    if new_idx == idx {
                        return None;
                    }
                    sub.nodes.swap(idx, new_idx);
                    Some(())
                })
                .is_some()
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                }
            }
            SubscriptionsMsg::DeleteSubscription(id) => {
                if commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                    let initial_len = subscriptions.len();
                    subscriptions.retain(|s| s.id != id);
                    (subscriptions.len() != initial_len).then_some(())
                })
                .is_some()
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    self.expanded_subs.remove(&id);
                    subscriptions_changed = true;
                }
            }
            SubscriptionsMsg::AddSubscription(name, source_input) => {
                let source = match source_input {
                    SubscriptionSourceInput::Url(url) => SubscriptionSource::Url { url },
                    SubscriptionSourceInput::File(path) => SubscriptionSource::File { path },
                    SubscriptionSourceInput::Paste(json) => {
                        match write_subscription_blob(&self.paths, &json) {
                            Ok(path) => SubscriptionSource::File {
                                path: path.to_string_lossy().into_owned(),
                            },
                            Err(err) => {
                                let _ = sender.output(SubscriptionsOutput::Notice(format!(
                                    "Failed to save pasted subscription: {err}"
                                )));
                                return;
                            }
                        }
                    }
                };
                let svc = self.service.clone();
                sender.oneshot_command(async move {
                    match svc.add_and_fetch(name, source).await {
                        Ok(outcome) => SubscriptionsCmdOutput::AddDone(outcome),
                        Err(e) => SubscriptionsCmdOutput::AddFailed(e),
                    }
                });
                return;
            }
            SubscriptionsMsg::UpdateSubscription(id) => {
                if !should_start_refresh(&self.refreshing, id) {
                    return;
                }
                let svc = self.service.clone();
                let subscription = match self.subscriptions.iter().find(|s| s.id == id) {
                    Some(subscription) => subscription.clone(),
                    None => return,
                };
                self.refreshing.insert(id);
                sender.oneshot_command(async move {
                    match svc.refresh(subscription).await {
                        Ok((sub, result)) => SubscriptionsCmdOutput::RefreshDone(sub, result),
                        Err(e) => SubscriptionsCmdOutput::RefreshFailed(id, e),
                    }
                });
                return;
            }
            SubscriptionsMsg::TestLatency(id) => {
                if self.testing_latency.contains(&id) || self.tun_active {
                    return;
                }
                let sub = match self.subscriptions.iter().find(|s| s.id == id) {
                    Some(s) => s.clone(),
                    None => return,
                };
                self.testing_latency.insert(id);
                let nodes = sub.nodes.clone();
                sender.oneshot_command(async move {
                    let (snapshot_ids, results) = run_latency_probe(nodes).await;
                    SubscriptionsCmdOutput::LatencyResult(id, snapshot_ids, results)
                });
            }
            SubscriptionsMsg::SortByLatency(id) => {
                if commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                    let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                    sub.nodes.sort_by(|a, b| {
                        let la = a.last_latency_ms.unwrap_or(u64::MAX);
                        let lb = b.last_latency_ms.unwrap_or(u64::MAX);
                        la.cmp(&lb)
                    });
                    Some(())
                })
                .is_some()
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                }
            }
            SubscriptionsMsg::TestRealDelay(id) => {
                if self.testing_real_delay.contains_key(&id) {
                    return;
                }
                if !self.real_delay_settings.enabled {
                    let _ = sender.output(SubscriptionsOutput::Notice(
                        "Real Delay testing is disabled in Preferences".to_string(),
                    ));
                    return;
                }
                let sub = match self.subscriptions.iter().find(|s| s.id == id) {
                    Some(s) => s.clone(),
                    None => return,
                };
                let Some(binary) = &self.binary_path else {
                    let _ = sender.output(SubscriptionsOutput::Notice(
                        "No backend binary configured".to_string(),
                    ));
                    return;
                };
                let node_refs: Vec<v2ray_rs_core::models::SubscriptionNode> = sub.nodes.clone();
                let backend_type = self.backend_type;
                let binary = binary.clone();
                let real_delay_settings = self.real_delay_settings.clone();
                let paths = self.paths.clone();
                let node_count = node_refs.len();
                self.real_delay_run_token = self.real_delay_run_token.wrapping_add(1);
                let run_token = self.real_delay_run_token;
                self.testing_real_delay.insert(id, run_token);
                let snapshot_ids: Vec<Uuid> = node_refs.iter().map(|n| n.id).collect();
                sender.oneshot_command(async move {
                    let timeout = std::time::Duration::from_millis(
                        u64::from(real_delay_settings.timeout_ms) + 15_000,
                    );
                    let report = match tokio::time::timeout(timeout, async {
                        let node_ref_ptrs: Vec<&v2ray_rs_core::models::SubscriptionNode> =
                            node_refs.iter().collect();
                        v2ray_rs_subscription::measure_real_delay(
                            backend_type,
                            &binary,
                            &node_ref_ptrs,
                            &real_delay_settings,
                            &paths,
                        )
                        .await
                    })
                    .await
                    {
                        Ok(report) => report,
                        Err(_) => RealDelayReport {
                            results: vec![None; node_count],
                            egress: vec![None; node_count],
                            verdicts: vec![HashMap::new(); node_count],
                            diagnostic: Some("Real Delay probe timed out".to_string()),
                        },
                    };
                    SubscriptionsCmdOutput::RealDelayResult(id, run_token, snapshot_ids, report)
                });
            }
            SubscriptionsMsg::CheckAvailability(id) => {
                if self.testing_availability.contains_key(&id) {
                    return;
                }
                if !self.real_delay_settings.enabled {
                    let _ = sender.output(SubscriptionsOutput::Notice(
                        "Real Delay testing is disabled in Preferences".to_string(),
                    ));
                    return;
                }
                let sub = match self.subscriptions.iter().find(|s| s.id == id) {
                    Some(s) => s.clone(),
                    None => return,
                };
                let Some(binary) = &self.binary_path else {
                    let _ = sender.output(SubscriptionsOutput::Notice(
                        "No backend binary configured".to_string(),
                    ));
                    return;
                };
                let node_refs: Vec<v2ray_rs_core::models::SubscriptionNode> = sub.nodes.clone();
                let backend_type = self.backend_type;
                let binary = binary.clone();
                let real_delay_settings = self.real_delay_settings.clone();
                let paths = self.paths.clone();
                let node_count = node_refs.len();
                let services: Vec<ServiceCheck> = builtin_service_checks().to_vec();
                let service_count = services.len();
                self.availability_run_token = self.availability_run_token.wrapping_add(1);
                let run_token = self.availability_run_token;
                self.testing_availability.insert(id, run_token);
                let snapshot_ids: Vec<Uuid> = node_refs.iter().map(|n| n.id).collect();
                sender.oneshot_command(async move {
                    // Same per-node pass as Real Delay, plus one request per
                    // service per node through its tunnel.
                    let timeout = std::time::Duration::from_millis(
                        u64::from(real_delay_settings.timeout_ms)
                            + 15_000
                            + 30_000 * service_count as u64,
                    );
                    let report = match tokio::time::timeout(timeout, async {
                        let node_ref_ptrs: Vec<&v2ray_rs_core::models::SubscriptionNode> =
                            node_refs.iter().collect();
                        v2ray_rs_subscription::check_node_availability(
                            backend_type,
                            &binary,
                            &node_ref_ptrs,
                            &real_delay_settings,
                            &paths,
                            &services,
                        )
                        .await
                    })
                    .await
                    {
                        Ok(report) => report,
                        Err(_) => RealDelayReport {
                            results: vec![None; node_count],
                            egress: vec![None; node_count],
                            verdicts: vec![HashMap::new(); node_count],
                            diagnostic: Some("Availability probe timed out".to_string()),
                        },
                    };
                    SubscriptionsCmdOutput::AvailabilityResult(id, run_token, snapshot_ids, report)
                });
            }
            SubscriptionsMsg::ShowAvailability(id) => {
                let sub = match self.subscriptions.iter().find(|s| s.id == id) {
                    Some(s) => s.clone(),
                    None => return,
                };
                show_availability_dialog(&sub, &self.verdicts, &self.egress_samples);
                return;
            }
            SubscriptionsMsg::SortByRealDelay(id) => {
                if commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                    let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                    sub.nodes.sort_by(|a, b| {
                        let la = a.last_real_delay_ms.unwrap_or(u64::MAX);
                        let lb = b.last_real_delay_ms.unwrap_or(u64::MAX);
                        la.cmp(&lb)
                    });
                    Some(())
                })
                .is_some()
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                }
            }
            SubscriptionsMsg::EnableAllNodes(id) => {
                if commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                    let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                    for node in &mut sub.nodes {
                        node.enabled = true;
                    }
                    Some(())
                })
                .is_some()
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                }
            }
            SubscriptionsMsg::DisableAllNodes(id) => {
                if commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                    let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                    for node in &mut sub.nodes {
                        node.enabled = false;
                    }
                    Some(())
                })
                .is_some()
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                }
            }
            SubscriptionsMsg::DragDropSubscription(from, to) => {
                if commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                    if from == to || from >= subscriptions.len() || to >= subscriptions.len() {
                        return None;
                    }
                    let sub = subscriptions.remove(from);
                    subscriptions.insert(to, sub);
                    Some(())
                })
                .is_some()
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                }
            }
            SubscriptionsMsg::DragDropNode(sub_id, from, to) => {
                if commit_subscriptions_mutation(&mut self.subscriptions, |subscriptions| {
                    let sub = subscriptions.iter_mut().find(|s| s.id == sub_id)?;
                    if from == to || from >= sub.nodes.len() || to >= sub.nodes.len() {
                        return None;
                    }
                    let node = sub.nodes.remove(from);
                    sub.nodes.insert(to, node);
                    Some(())
                })
                .is_some()
                {
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    subscriptions_changed = true;
                }
            }
            SubscriptionsMsg::SetActiveNode(active) => {
                if self.active_node == active {
                    return;
                }
                self.active_node = active;
            }
            SubscriptionsMsg::SetTunActive(active) => {
                if self.tun_active == active {
                    return;
                }
                self.tun_active = active;
            }
            SubscriptionsMsg::ConnectNode(sub_id, node_id) => {
                let _ = sender.output(SubscriptionsOutput::ConnectNode(sub_id, node_id));
            }
            SubscriptionsMsg::ResetStorage => match self.store.reset_subscriptions() {
                Ok(()) => {
                    self.subscriptions.clear();
                    self.load_error = None;
                    subscriptions_changed = true;
                }
                Err(err) => {
                    log::error!("reset subscriptions: {err}");
                }
            },
            SubscriptionsMsg::CheckAutoUpdate => {
                let svc = self.service.clone();
                let interval = self.auto_update_interval_secs;
                let subscriptions = self.subscriptions.clone();
                sender.oneshot_command(async move {
                    let results = svc.refresh_all_overdue(subscriptions, interval).await;
                    let mapped: Vec<_> = results
                        .into_iter()
                        .map(|(id, r)| (id, r.map_err(|e| e.to_string())))
                        .collect();
                    SubscriptionsCmdOutput::AutoUpdateDone(mapped)
                });
                return;
            }
            SubscriptionsMsg::SyncSettings {
                auto_update_enabled,
                auto_update_interval_secs,
                backend_type,
                binary_path,
                real_delay_settings,
            } => {
                let backend_changed = self.backend_type != backend_type;
                let binary_changed = self.binary_path != binary_path;
                let changed = self.auto_update_enabled != auto_update_enabled
                    || self.auto_update_interval_secs != auto_update_interval_secs;
                self.auto_update_enabled = auto_update_enabled;
                self.auto_update_interval_secs = auto_update_interval_secs;
                self.backend_type = backend_type;
                self.binary_path = binary_path;
                self.real_delay_settings = real_delay_settings;
                if backend_changed || binary_changed {
                    self.real_delay_run_token = self.real_delay_run_token.wrapping_add(1);
                    self.testing_real_delay.clear();
                    self.real_delay_capability = backend_type.default_real_delay_capability();
                    render_hint = RenderHint::Full;
                }

                if changed {
                    self.auto_update_generation = self.auto_update_generation.wrapping_add(1);
                    if self.auto_update_enabled {
                        sender.input(SubscriptionsMsg::CheckAutoUpdate);
                        schedule_auto_update_tick(
                            sender.clone(),
                            self.auto_update_generation,
                            self.auto_update_interval_secs,
                        );
                    }
                }
            }
            SubscriptionsMsg::ExpanderToggled(id, expanded) => {
                if expanded {
                    self.expanded_subs.insert(id);
                } else {
                    self.expanded_subs.remove(&id);
                }
                return;
            }
        }
        if subscriptions_changed {
            let _ = sender.output(SubscriptionsOutput::SubscriptionsChanged);
        }
        emit_active_nodes(&self.subscriptions, &sender);

        match render_hint {
            RenderHint::NodeToggle(sub_id, idx) => {
                if update_node_switch(&self.list_container, sub_id, idx, &self.subscriptions) {
                    return;
                }
            }
            RenderHint::SubscriptionToggle(id) => {
                if update_subscription_toggle(&self.list_container, id, &self.subscriptions) {
                    return;
                }
            }
            RenderHint::SubscriptionRename(id) => {
                if update_subscription_title(&self.list_container, id, &self.subscriptions) {
                    return;
                }
            }
            RenderHint::Full => {}
        }

        render_list(
            &self.subscriptions,
            &self.list_container,
            &sender,
            RenderState {
                expanded_subs: &self.expanded_subs,
                testing_latency: &self.testing_latency,
                testing_real_delay: &self.testing_real_delay,
                real_delay_available: self.real_delay_settings.enabled
                    && self.backend_type.supports_real_delay(),
                real_delay_capability: &self.real_delay_capability,
                locked: self.load_error.is_some(),
                active_node: self.active_node,
                tun_active: self.tun_active,
                egress_samples: &self.egress_samples,
                verdicts: &self.verdicts,
                testing_availability: &self.testing_availability,
            },
        );
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            SubscriptionsCmdOutput::AddDone(outcome) => {
                let summary =
                    format_update_summary(&outcome.subscription.name, "Imported", &outcome.result);
                self.subscriptions.push(outcome.subscription);
                self.persister
                    .submit(&self.store, &sender, self.subscriptions.clone());
                let _ = sender.output(SubscriptionsOutput::SubscriptionsChanged);
                let _ = sender.output(SubscriptionsOutput::Notice(summary));
            }
            SubscriptionsCmdOutput::RefreshDone(sub, result) => {
                let id = sub.id;
                self.refreshing.remove(&id);
                if let Some(pos) = self.subscriptions.iter().position(|s| s.id == id) {
                    let name = self.subscriptions[pos].name.clone();
                    let previous = self.subscriptions[pos].clone();
                    let lost_profile =
                        previous.imported_profile.is_some() && sub.imported_profile.is_none();
                    let merged = merge_refreshed_subscription(&previous, sub);
                    let runtime_changed = !previous.runtime_state_eq(&merged);
                    self.subscriptions[pos] = merged;
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    let _ = sender.output(SubscriptionsOutput::Notice(format_update_summary(
                        &name, "Updated", &result,
                    )));
                    if runtime_changed {
                        let _ = sender.output(SubscriptionsOutput::SubscriptionsChanged);
                    }
                    if lost_profile {
                        let _ = sender.output(SubscriptionsOutput::Notice(format!(
                            "{name}: source no longer carries a routing profile — keeping the existing one"
                        )));
                    }
                }
                let name = self
                    .subscriptions
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.name.as_str())
                    .unwrap_or("unknown");
                log::info!(
                    "updated subscription {name}: +{} -{} ={} failed={}",
                    result.added,
                    result.removed,
                    result.unchanged,
                    result.parse_failures.len()
                );
            }
            SubscriptionsCmdOutput::LatencyResult(id, snapshot_ids, results) => {
                self.testing_latency.remove(&id);
                let mut latency_updates = Vec::new();
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    apply_latency_by_ids(sub, &snapshot_ids, &results);
                    for (idx, latency) in results.iter().enumerate() {
                        if let (Some(node_id), Some(value)) =
                            (snapshot_ids.get(idx).copied(), latency)
                        {
                            latency_updates.push((node_id, *value));
                        }
                    }
                }
                let now = chrono::Utc::now();
                persist_latency_snapshot(&sender, &self.store, move |snapshot| {
                    for (node_id, value) in latency_updates {
                        snapshot.upsert(
                            ConnectionNodeRef::Subscription {
                                subscription_id: id,
                                node_id,
                            },
                            value,
                            now,
                        );
                    }
                });
            }
            SubscriptionsCmdOutput::RealDelayResult(id, run_token, snapshot_ids, report) => {
                if self.testing_real_delay.get(&id).copied() != Some(run_token) {
                    return;
                }
                self.testing_real_delay.remove(&id);
                let now = chrono::Utc::now();
                let mut delay_updates = Vec::new();
                let mut egress_updates = Vec::new();
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    apply_real_delay_by_ids(sub, &snapshot_ids, &report.results);
                    for idx in 0..snapshot_ids.len() {
                        let node_id = snapshot_ids[idx];
                        if let Some(value) = report.results.get(idx).copied().flatten() {
                            delay_updates.push((node_id, value));
                        }
                        if let Some(sample) = report.egress.get(idx).and_then(|e| e.as_ref()) {
                            if let Some(node) = node_by_snapshot_id(sub, &snapshot_ids, idx) {
                                self.egress_samples.insert((id, node.id), sample.clone());
                            }
                            egress_updates.push((node_id, sample.clone()));
                        }
                    }
                }
                persist_latency_snapshot(&sender, &self.store, move |snapshot| {
                    for (node_id, value) in delay_updates {
                        snapshot.upsert_real_delay(
                            ConnectionNodeRef::Subscription {
                                subscription_id: id,
                                node_id,
                            },
                            value,
                            now,
                        );
                    }
                    for (node_id, sample) in egress_updates {
                        snapshot.upsert_egress(
                            ConnectionNodeRef::Subscription {
                                subscription_id: id,
                                node_id,
                            },
                            sample.address.clone(),
                            sample.location.clone(),
                            sample.checked_at,
                        );
                    }
                });
                // Update capability based on results for Xray/V2ray backends
                if matches!(self.backend_type, BackendType::Xray | BackendType::V2ray) {
                    let has_any_result = report.results.iter().any(|r| r.is_some());
                    self.real_delay_capability = if has_any_result {
                        RealDelayCapability::Supported
                    } else {
                        // If we have a diagnostic about missing service, mark as unsupported
                        if let Some(diag) = &report.diagnostic {
                            if diag.contains("ObservatoryService") {
                                RealDelayCapability::Unsupported {
                                    reason: diag.clone(),
                                }
                            } else {
                                self.real_delay_capability.clone()
                            }
                        } else {
                            self.real_delay_capability.clone()
                        }
                    };
                }

                if let Some(diagnostic) = report.diagnostic {
                    let _ = sender.output(SubscriptionsOutput::Notice(diagnostic));
                }
            }
            SubscriptionsCmdOutput::AvailabilityResult(id, run_token, snapshot_ids, report) => {
                if self.testing_availability.get(&id).copied() != Some(run_token) {
                    return;
                }
                self.testing_availability.remove(&id);
                let now = chrono::Utc::now();
                let mut delay_updates = Vec::new();
                let mut egress_updates = Vec::new();
                let mut verdict_updates = Vec::new();
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    apply_real_delay_by_ids(sub, &snapshot_ids, &report.results);
                    for idx in 0..snapshot_ids.len() {
                        let node_id = snapshot_ids[idx];
                        if let Some(value) = report.results.get(idx).copied().flatten() {
                            delay_updates.push((node_id, value));
                        }
                        if let Some(sample) = report.egress.get(idx).and_then(|e| e.as_ref()) {
                            if let Some(node) = node_by_snapshot_id(sub, &snapshot_ids, idx) {
                                self.egress_samples.insert((id, node.id), sample.clone());
                            }
                            egress_updates.push((node_id, sample.clone()));
                        }
                        if let Some(verdict_map) =
                            report.verdicts.get(idx).filter(|v| !v.is_empty())
                        {
                            if let Some(node) = node_by_snapshot_id(sub, &snapshot_ids, idx) {
                                self.verdicts.insert((id, node.id), verdict_map.clone());
                            }
                            verdict_updates.push((node_id, verdict_map.clone()));
                        }
                    }
                }
                persist_latency_snapshot(&sender, &self.store, move |snapshot| {
                    for (node_id, value) in delay_updates {
                        snapshot.upsert_real_delay(
                            ConnectionNodeRef::Subscription {
                                subscription_id: id,
                                node_id,
                            },
                            value,
                            now,
                        );
                    }
                    for (node_id, sample) in egress_updates {
                        snapshot.upsert_egress(
                            ConnectionNodeRef::Subscription {
                                subscription_id: id,
                                node_id,
                            },
                            sample.address.clone(),
                            sample.location.clone(),
                            sample.checked_at,
                        );
                    }
                    for (node_id, verdicts) in verdict_updates {
                        snapshot.upsert_verdicts(
                            ConnectionNodeRef::Subscription {
                                subscription_id: id,
                                node_id,
                            },
                            verdicts,
                            now,
                        );
                    }
                });
                if let Some(sub) = self.subscriptions.iter().find(|s| s.id == id) {
                    show_availability_dialog(sub, &self.verdicts, &self.egress_samples);
                }
                if let Some(diagnostic) = report.diagnostic {
                    let _ = sender.output(SubscriptionsOutput::Notice(diagnostic));
                }
            }
            SubscriptionsCmdOutput::RefreshFailed(id, error) => {
                self.refreshing.remove(&id);
                let name = self
                    .subscriptions
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.name.as_str())
                    .unwrap_or("unknown");
                log::error!("failed to update subscription {name}: {error}");
                let _ = sender.output(SubscriptionsOutput::Notice(format_subscription_error(
                    &error,
                )));
            }
            SubscriptionsCmdOutput::AddFailed(error) => {
                log::error!("failed to add subscription: {error}");
                let _ = sender.output(SubscriptionsOutput::Notice(format_subscription_error(
                    &error,
                )));
            }
            SubscriptionsCmdOutput::AutoUpdateDone(results) => {
                self.auto_update_streaks
                    .retain(|id, _| self.subscriptions.iter().any(|s| s.id == *id));
                if !results.is_empty() {
                    let previous = self.subscriptions.clone();
                    for (_, result) in &results {
                        if let Ok((updated, _)) = result
                            && let Some(existing) =
                                self.subscriptions.iter_mut().find(|s| s.id == updated.id)
                        {
                            *existing = merge_refreshed_subscription(existing, updated.clone());
                        }
                    }
                    let runtime_changed =
                        !subscriptions_runtime_state_eq(&previous, &self.subscriptions);
                    self.persister
                        .submit(&self.store, &sender, self.subscriptions.clone());
                    if runtime_changed {
                        let _ = sender.output(SubscriptionsOutput::SubscriptionsChanged);
                    }
                    for (id, result) in &results {
                        let name = self
                            .subscriptions
                            .iter()
                            .find(|s| s.id == *id)
                            .map(|s| s.name.as_str())
                            .unwrap_or("unknown");
                        match result {
                            Ok((_, r)) => {
                                log::info!(
                                    "auto-updated {name}: +{} -{} ={}",
                                    r.added,
                                    r.removed,
                                    r.unchanged
                                );
                                auto_update_outcome_toast(
                                    &mut self.auto_update_streaks,
                                    *id,
                                    name,
                                    result,
                                );
                            }
                            Err(e) => {
                                log::warn!("auto-update {name} failed: {e}");
                                if let Some(toast) = auto_update_outcome_toast(
                                    &mut self.auto_update_streaks,
                                    *id,
                                    name,
                                    result,
                                ) {
                                    let _ = sender.output(SubscriptionsOutput::Notice(toast));
                                }
                            }
                        }
                    }
                }
            }
            SubscriptionsCmdOutput::SubscriptionsSaved(result) => {
                if let Err(err) = result {
                    report_subscription_persist_error(&sender, &err);
                }
            }
            SubscriptionsCmdOutput::SnapshotSaved(result) => {
                if let Err(err) = result {
                    log::error!("save latency snapshot: {err}");
                }
            }
            SubscriptionsCmdOutput::AutoUpdateTick(generation) => {
                if self.auto_update_enabled && generation == self.auto_update_generation {
                    sender.input(SubscriptionsMsg::CheckAutoUpdate);
                    schedule_auto_update_tick(
                        sender.clone(),
                        generation,
                        self.auto_update_interval_secs,
                    );
                }
            }
            SubscriptionsCmdOutput::BackgroundLatencyTick => {
                let eligible = subscriptions_eligible_for_latency_test(
                    &self.subscriptions,
                    &self.testing_latency,
                    self.tun_active,
                );
                if self.tun_active {
                    log::debug!("skipping background latency probes: TUN active");
                }
                for id in eligible {
                    sender.input(SubscriptionsMsg::TestLatency(id));
                }
                sender.oneshot_command(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                    SubscriptionsCmdOutput::BackgroundLatencyTick
                });
            }
        }
        let has_active = self.subscriptions.iter().any(|s| s.has_enabled_nodes());
        let _ = sender.output(SubscriptionsOutput::ActiveNodesChanged(has_active));
        render_list(
            &self.subscriptions,
            &self.list_container,
            &sender,
            RenderState {
                expanded_subs: &self.expanded_subs,
                testing_latency: &self.testing_latency,
                testing_real_delay: &self.testing_real_delay,
                real_delay_available: self.real_delay_settings.enabled
                    && self.backend_type.supports_real_delay(),
                real_delay_capability: &self.real_delay_capability,
                locked: self.load_error.is_some(),
                active_node: self.active_node,
                tun_active: self.tun_active,
                egress_samples: &self.egress_samples,
                verdicts: &self.verdicts,
                testing_availability: &self.testing_availability,
            },
        );
    }
}

fn auto_update_outcome_toast(
    streaks: &mut HashMap<Uuid, FailureStreak>,
    id: Uuid,
    name: &str,
    result: &AutoUpdateRefreshResult,
) -> Option<String> {
    match result {
        Ok(_) => {
            if let Some(streak) = streaks.get_mut(&id) {
                streak.record_success();
            }
            None
        }
        Err(err) => streaks
            .entry(id)
            .or_default()
            .record_failure()
            .then(|| format!("Auto-update failed for {name}: {err}")),
    }
}

fn schedule_auto_update_tick(
    sender: ComponentSender<SubscriptionsPage>,
    generation: u64,
    interval_secs: u64,
) {
    sender.oneshot_command(async move {
        tokio::time::sleep(std::time::Duration::from_secs(interval_secs.max(60))).await;
        SubscriptionsCmdOutput::AutoUpdateTick(generation)
    });
}

fn merge_refreshed_subscription(current: &Subscription, refreshed: Subscription) -> Subscription {
    let nodes = reconcile_nodes(
        &current.nodes,
        refreshed.nodes.into_iter().map(|node| node.node).collect(),
    );

    Subscription {
        id: current.id,
        name: current.name.clone(),
        source: current.source.clone(),
        nodes,
        last_updated: refreshed.last_updated,
        auto_update_interval_secs: current.auto_update_interval_secs,
        enabled: current.enabled,
        imported_profile: refreshed
            .imported_profile
            .or_else(|| current.imported_profile.clone()),
        use_imported_profile: current.use_imported_profile,
    }
}

fn redact_uri(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return "unrecognized entry".to_string();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    };
    if host.is_empty() {
        "unrecognized entry".to_string()
    } else {
        format!("{scheme}://{host}")
    }
}

fn format_update_summary(name: &str, action: &str, result: &UpdateResult) -> String {
    let mut summary = if result.parse_failures.is_empty() {
        format!(
            "{action} {name}: +{} -{} unchanged {}",
            result.added, result.removed, result.unchanged
        )
    } else {
        let failed = result
            .parse_failures
            .iter()
            .take(2)
            .enumerate()
            .map(|(idx, failure)| format!("line {}: {}", idx + 1, redact_uri(&failure.uri)))
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if result.parse_failures.len() > 2 {
            ", ..."
        } else {
            ""
        };
        format!(
            "{action} {name}: +{} -{} unchanged {}, {} invalid URI(s): {failed}{suffix}",
            result.added,
            result.removed,
            result.unchanged,
            result.parse_failures.len()
        )
    };

    if let Some(profile) = &result.profile
        && !profile.skipped.is_empty()
    {
        let skipped = profile
            .skipped
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let suffix = if profile.skipped.len() > 2 {
            "; ..."
        } else {
            ""
        };
        summary.push_str(&format!(
            "; {} profile item(s) skipped: {skipped}{suffix}",
            profile.skipped.len()
        ));
    }

    summary
}

fn format_subscription_error(error: &SubscriptionError) -> String {
    match error {
        SubscriptionError::Update(UpdateError::InvalidContent { failures }) => {
            if failures.is_empty() {
                "Subscription import failed: no valid proxy URIs were found".to_string()
            } else {
                let details = failures
                    .iter()
                    .take(2)
                    .enumerate()
                    .map(|(idx, failure)| format!("line {}: {}", idx + 1, redact_uri(&failure.uri)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let suffix = if failures.len() > 2 { ", ..." } else { "" };
                format!(
                    "Subscription import failed: no valid proxy URIs. Failed entries: {details}{suffix}"
                )
            }
        }
        _ => error.to_string(),
    }
}

fn subscriptions_eligible_for_latency_test(
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

fn update_node_switch(
    container: &gtk::ListBox,
    sub_id: Uuid,
    node_idx: usize,
    subscriptions: &[Subscription],
) -> bool {
    let sub = match subscriptions.iter().find(|s| s.id == sub_id) {
        Some(s) => s,
        None => return false,
    };
    let node = match sub.nodes.get(node_idx) {
        Some(n) => n,
        None => return false,
    };

    let mut child = container.first_child();
    while let Some(ref widget) = child {
        if let Some(expander) = widget.downcast_ref::<adw::ExpanderRow>() {
            let name = expander.widget_name();
            if let Ok(id) = Uuid::parse_str(&name)
                && id == sub_id
            {
                let mut current_idx = 0;
                let mut inner_child = expander.first_child();
                while let Some(ref inner_widget) = inner_child {
                    if inner_widget.is::<adw::ActionRow>() {
                        if current_idx == node_idx {
                            if let Some(action_row) = inner_widget.downcast_ref::<adw::ActionRow>()
                            {
                                let mut descendant = action_row.first_child();
                                while let Some(ref desc_widget) = descendant {
                                    if let Some(switch) = desc_widget.downcast_ref::<gtk::Switch>()
                                    {
                                        if switch.is_active() != node.enabled {
                                            switch.set_active(node.enabled);
                                        }
                                        action_row.set_opacity(if node.enabled {
                                            1.0
                                        } else {
                                            0.5
                                        });
                                        return true;
                                    }
                                    if let Some(container) =
                                        desc_widget.downcast_ref::<gtk::Widget>()
                                    {
                                        let mut nested = container.first_child();
                                        while let Some(ref nested_widget) = nested {
                                            if let Some(switch) =
                                                nested_widget.downcast_ref::<gtk::Switch>()
                                            {
                                                if switch.is_active() != node.enabled {
                                                    switch.set_active(node.enabled);
                                                }
                                                action_row.set_opacity(if node.enabled {
                                                    1.0
                                                } else {
                                                    0.5
                                                });
                                                return true;
                                            }
                                            nested = nested_widget.next_sibling();
                                        }
                                    }
                                    descendant = desc_widget.next_sibling();
                                }
                            }
                            return false;
                        }
                        current_idx += 1;
                    }
                    inner_child = inner_widget.next_sibling();
                }
                return false;
            }
        }
        child = widget.next_sibling();
    }
    false
}

fn find_expander_by_id(container: &gtk::ListBox, sub_id: Uuid) -> Option<adw::ExpanderRow> {
    let mut child = container.first_child();
    while let Some(ref widget) = child {
        if let Some(expander) = widget.downcast_ref::<adw::ExpanderRow>()
            && let Ok(id) = Uuid::parse_str(&expander.widget_name())
            && id == sub_id
        {
            return Some(expander.clone());
        }
        child = widget.next_sibling();
    }
    None
}

fn update_subscription_toggle(
    container: &gtk::ListBox,
    sub_id: Uuid,
    subscriptions: &[Subscription],
) -> bool {
    let sub = match subscriptions.iter().find(|s| s.id == sub_id) {
        Some(s) => s,
        None => return false,
    };
    let Some(expander) = find_expander_by_id(container, sub_id) else {
        return false;
    };

    expander.set_opacity(if sub.enabled { 1.0 } else { 0.5 });

    let mut suffix = expander.first_child();
    while let Some(ref widget) = suffix {
        if let Some(switch) = widget.downcast_ref::<gtk::Switch>() {
            switch.set_active(sub.enabled);
            return true;
        }
        suffix = widget.next_sibling();
    }
    true
}

fn update_subscription_title(
    container: &gtk::ListBox,
    sub_id: Uuid,
    subscriptions: &[Subscription],
) -> bool {
    let sub = match subscriptions.iter().find(|s| s.id == sub_id) {
        Some(s) => s,
        None => return false,
    };
    let Some(expander) = find_expander_by_id(container, sub_id) else {
        return false;
    };

    let source_text = match &sub.source {
        SubscriptionSource::Url { url } => truncate(url, 50),
        SubscriptionSource::File { path } => path.clone(),
    };
    let updated_text = match &sub.last_updated {
        Some(dt) => format!("Updated: {}", dt.format("%Y-%m-%d %H:%M")),
        None => "Never updated".into(),
    };

    expander.set_title(&sub.name);
    expander.set_subtitle(&format!(
        "{} | {} nodes | {}",
        source_text,
        sub.nodes.len(),
        updated_text
    ));
    true
}

fn render_list(
    subs: &[Subscription],
    container: &gtk::ListBox,
    sender: &ComponentSender<SubscriptionsPage>,
    state: RenderState<'_>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    if subs.is_empty() {
        let empty = adw::StatusPage::builder()
            .icon_name("folder-download-symbolic")
            .title("No Subscriptions")
            .description("Add a subscription to get started")
            .build();
        let row = gtk::ListBoxRow::builder()
            .selectable(false)
            .activatable(false)
            .child(&empty)
            .build();
        container.append(&row);
        return;
    }

    for (idx, sub) in subs.iter().enumerate() {
        let expander = build_subscription_group(sub, idx, sender, &state);
        container.append(&expander);
    }
}

fn build_subscription_group(
    sub: &Subscription,
    sub_idx: usize,
    sender: &ComponentSender<SubscriptionsPage>,
    state: &RenderState<'_>,
) -> adw::ExpanderRow {
    let expander = build_expander_header(sub, state.expanded_subs.contains(&sub.id), sender);
    attach_drag_handle(&expander, sub_idx, sender, state.locked);
    attach_subscription_toggle(&expander, sub.id, sub.enabled, sender, state.locked);
    let menu = build_subscription_menu(sub, sender, state);
    expander.add_suffix(&menu);

    for (idx, node) in sub.nodes.iter().enumerate() {
        let active = state.active_node == Some((sub.id, node.id));
        let egress = state.egress_samples.get(&(sub.id, node.id));
        expander.add_row(&build_node_row(
            sub.id,
            idx,
            node,
            sender,
            state.locked,
            active,
            egress,
        ));
    }

    attach_profile_toggle(&expander, sub, sender, state.locked);

    expander
}

fn build_expander_header(
    sub: &Subscription,
    expanded: bool,
    sender: &ComponentSender<SubscriptionsPage>,
) -> adw::ExpanderRow {
    let source_text = match &sub.source {
        SubscriptionSource::Url { url } => truncate(url, 50),
        SubscriptionSource::File { path } => path.clone(),
    };
    let updated_text = match &sub.last_updated {
        Some(dt) => format!("Updated: {}", dt.format("%Y-%m-%d %H:%M")),
        None => "Never updated".into(),
    };

    let expander = adw::ExpanderRow::builder()
        .title(&sub.name)
        .subtitle(format!(
            "{} | {} nodes | {}",
            source_text,
            sub.nodes.len(),
            updated_text
        ))
        .show_enable_switch(false)
        .enable_expansion(true)
        .expanded(expanded)
        .build();

    expander.set_widget_name(&sub.id.to_string());
    if !sub.enabled {
        expander.set_opacity(0.5);
    }

    {
        let id = sub.id;
        let s = sender.clone();
        expander.connect_expanded_notify(move |exp| {
            s.input(SubscriptionsMsg::ExpanderToggled(id, exp.is_expanded()));
        });
    }

    expander
}

fn attach_drag_handle(
    expander: &adw::ExpanderRow,
    sub_idx: usize,
    sender: &ComponentSender<SubscriptionsPage>,
    locked: bool,
) {
    let handle = gtk::Image::builder()
        .icon_name("list-drag-handle-symbolic")
        .build();
    handle.add_css_class("dim-label");

    if !locked {
        let drag_source = gtk::DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE);
        {
            let idx = sub_idx;
            drag_source.connect_prepare(move |_src, _x, _y| {
                Some(gdk::ContentProvider::for_value(
                    &format!("sub_{idx}").to_value(),
                ))
            });
        }
        handle.add_controller(drag_source);

        let drop_target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        {
            let target_idx = sub_idx;
            let s = sender.clone();
            drop_target.connect_drop(move |_target, value, _x, _y| {
                if let Ok(val) = value.get::<String>()
                    && let Some(from_str) = val.strip_prefix("sub_")
                    && let Ok(from_idx) = from_str.parse::<usize>()
                {
                    s.input(SubscriptionsMsg::DragDropSubscription(from_idx, target_idx));
                    return true;
                }
                false
            });
        }
        expander.add_controller(drop_target);
    }
    expander.add_prefix(&handle);
}

fn attach_subscription_toggle(
    expander: &adw::ExpanderRow,
    sub_id: Uuid,
    enabled: bool,
    sender: &ComponentSender<SubscriptionsPage>,
    locked: bool,
) {
    let toggle = gtk::Switch::builder()
        .active(enabled)
        .valign(gtk::Align::Center)
        .sensitive(!locked)
        .build();
    {
        let s = sender.clone();
        toggle.connect_active_notify(move |_| {
            s.input(SubscriptionsMsg::ToggleSubscription(sub_id));
        });
    }
    expander.add_suffix(&toggle);
}

fn attach_profile_toggle(
    expander: &adw::ExpanderRow,
    sub: &Subscription,
    sender: &ComponentSender<SubscriptionsPage>,
    locked: bool,
) {
    let Some(profile) = &sub.imported_profile else {
        return;
    };

    let dns_servers = profile.dns.as_ref().map(|d| d.servers.len()).unwrap_or(0);
    let row = adw::SwitchRow::builder()
        .title("Use Provider Routing & DNS")
        .subtitle(format!(
            "{} rule(s), {} DNS server(s) from this subscription",
            profile.rules.len(),
            dns_servers
        ))
        .active(sub.use_imported_profile)
        .sensitive(!locked)
        .build();

    let id = sub.id;
    let s = sender.clone();
    row.connect_active_notify(move |_| {
        s.input(SubscriptionsMsg::ToggleUseImportedProfile(id));
    });

    expander.add_row(&row);
}

fn build_subscription_menu(
    sub: &Subscription,
    sender: &ComponentSender<SubscriptionsPage>,
    state: &RenderState<'_>,
) -> gtk::MenuButton {
    let is_testing = state.testing_latency.contains(&sub.id);
    let is_testing_real_delay = state.testing_real_delay.contains_key(&sub.id);
    let real_delay_available = state.real_delay_available;
    let real_delay_capability = state.real_delay_capability;
    let tun_active = state.tun_active;
    let is_testing_availability = state.testing_availability.contains_key(&sub.id);
    let has_availability = sub
        .nodes
        .iter()
        .any(|n| state.verdicts.contains_key(&(sub.id, n.id)));

    let menu_btn = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .valign(gtk::Align::Center)
        .has_frame(false)
        .build();
    menu_btn.add_css_class("flat");

    let popover = gtk::Popover::new();
    let popover_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();

    let id = sub.id;
    let has_latency = sub.nodes.iter().any(|n| n.last_latency_ms.is_some());

    let update_btn = gtk::Button::builder()
        .label("Update")
        .has_frame(false)
        .build();
    {
        let s = sender.clone();
        let p = popover.clone();
        update_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::UpdateSubscription(id));
        });
    }

    let rename_btn = gtk::Button::builder()
        .label("Rename")
        .has_frame(false)
        .build();
    {
        let current_name = sub.name.clone();
        let s = sender.clone();
        let p = popover.clone();
        rename_btn.connect_clicked(move |_| {
            p.popdown();
            show_rename_dialog(id, &current_name, s.clone());
        });
    }

    let delete_btn = gtk::Button::builder()
        .label("Delete")
        .has_frame(false)
        .build();
    delete_btn.add_css_class("destructive-action");
    {
        let s = sender.clone();
        let p = popover.clone();
        delete_btn.connect_clicked(move |_| {
            p.popdown();
            show_delete_dialog(id, s.clone());
        });
    }

    let move_up_btn = gtk::Button::builder()
        .label("Move Up")
        .has_frame(false)
        .build();
    {
        let s = sender.clone();
        let p = popover.clone();
        move_up_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::MoveSubscription(id, Direction::Up));
        });
    }

    let move_down_btn = gtk::Button::builder()
        .label("Move Down")
        .has_frame(false)
        .build();
    {
        let s = sender.clone();
        let p = popover.clone();
        move_down_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::MoveSubscription(id, Direction::Down));
        });
    }

    let test_latency_btn = gtk::Button::builder()
        .label(if is_testing {
            "Testing..."
        } else {
            "Test Latency"
        })
        .has_frame(false)
        .sensitive(!is_testing && !tun_active)
        .build();
    if tun_active {
        test_latency_btn.set_tooltip_text(Some(
            "Direct TCP latency is unavailable under TUN — use Test Real Delay to measure through the tunnel",
        ));
    }
    {
        let s = sender.clone();
        let p = popover.clone();
        test_latency_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::TestLatency(id));
        });
    }

    let sort_latency_btn = gtk::Button::builder()
        .label("Sort by Latency")
        .has_frame(false)
        .sensitive(has_latency)
        .build();
    {
        let s = sender.clone();
        let p = popover.clone();
        sort_latency_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::SortByLatency(id));
        });
    }

    let enable_all_btn = gtk::Button::builder()
        .label("Enable All Nodes")
        .has_frame(false)
        .build();
    {
        let s = sender.clone();
        let p = popover.clone();
        enable_all_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::EnableAllNodes(id));
        });
    }

    let disable_all_btn = gtk::Button::builder()
        .label("Disable All Nodes")
        .has_frame(false)
        .build();
    {
        let s = sender.clone();
        let p = popover.clone();
        disable_all_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::DisableAllNodes(id));
        });
    }

    popover_box.append(&update_btn);
    popover_box.append(&rename_btn);
    popover_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    popover_box.append(&test_latency_btn);
    popover_box.append(&sort_latency_btn);

    let has_real_delay = sub.nodes.iter().any(|n| n.last_real_delay_ms.is_some());

    // Determine button sensitivity and tooltip based on capability
    let (btn_sensitive, btn_tooltip) = match real_delay_capability {
        RealDelayCapability::Supported => (true, None),
        RealDelayCapability::PotentiallySupported { requirement } => (
            true,
            Some(format!(
                "{} availability is checked when the probe runs",
                requirement
            )),
        ),
        RealDelayCapability::Unsupported { reason } => (false, Some(reason.clone())),
    };

    let test_real_delay_btn = gtk::Button::builder()
        .label(if is_testing_real_delay {
            "Testing Real Delay..."
        } else {
            "Test Real Delay"
        })
        .has_frame(false)
        .sensitive(!is_testing_real_delay && real_delay_available && btn_sensitive)
        .build();
    if let Some(tooltip) = btn_tooltip {
        test_real_delay_btn.set_tooltip_text(Some(&tooltip));
    } else if !real_delay_available {
        test_real_delay_btn.set_tooltip_text(Some(
            "Real Delay is not available (check settings and backend)",
        ));
    }
    {
        let s = sender.clone();
        let p = popover.clone();
        test_real_delay_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::TestRealDelay(id));
        });
    }
    popover_box.append(&test_real_delay_btn);

    let sort_real_delay_btn = gtk::Button::builder()
        .label("Sort by Real Delay")
        .has_frame(false)
        .sensitive(has_real_delay)
        .build();
    {
        let s = sender.clone();
        let p = popover.clone();
        sort_real_delay_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::SortByRealDelay(id));
        });
    }
    popover_box.append(&sort_real_delay_btn);

    // Availability probe: same backend-support rules as Real Delay, since it
    // rides the same ephemeral session and per-node inbounds.
    let check_availability_btn = gtk::Button::builder()
        .label(if is_testing_availability {
            "Checking Availability..."
        } else {
            "Check Availability"
        })
        .has_frame(false)
        .sensitive(!is_testing_availability && real_delay_available && btn_sensitive && !tun_active)
        .build();
    if !real_delay_available || !btn_sensitive {
        check_availability_btn.set_tooltip_text(Some(
            "Availability checks use the same ephemeral backend session as Real Delay",
        ));
    }
    {
        let s = sender.clone();
        let p = popover.clone();
        check_availability_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::CheckAvailability(id));
        });
    }
    popover_box.append(&check_availability_btn);

    let availability_results_btn = gtk::Button::builder()
        .label("Availability Results")
        .has_frame(false)
        .sensitive(has_availability)
        .build();
    {
        let s = sender.clone();
        let p = popover.clone();
        availability_results_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::ShowAvailability(id));
        });
    }
    popover_box.append(&availability_results_btn);

    popover_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    popover_box.append(&enable_all_btn);
    popover_box.append(&disable_all_btn);
    popover_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    popover_box.append(&move_up_btn);
    popover_box.append(&move_down_btn);
    popover_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    popover_box.append(&delete_btn);
    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));

    menu_btn
}

fn build_node_row(
    sub_id: Uuid,
    idx: usize,
    node: &v2ray_rs_core::models::SubscriptionNode,
    sender: &ComponentSender<SubscriptionsPage>,
    locked: bool,
    active: bool,
    egress: Option<&EgressSample>,
) -> adw::ActionRow {
    let protocol = match &node.node {
        v2ray_rs_core::models::ProxyNode::Vless(_) => "VLESS",
        v2ray_rs_core::models::ProxyNode::Vmess(_) => "VMESS",
        v2ray_rs_core::models::ProxyNode::Shadowsocks(_) => "SS",
        v2ray_rs_core::models::ProxyNode::Trojan(_) => "TROJAN",
    };

    let address = format!("{}:{}", node.node.address(), node.node.port());
    let name = node.node.remark().unwrap_or("Unnamed Node");

    let row = adw::ActionRow::builder()
        .title(name)
        .subtitle(format_node_subtitle(&address, egress))
        .build();

    if !node.enabled {
        row.set_opacity(0.5);
    }

    let node_handle = gtk::Image::builder()
        .icon_name("list-drag-handle-symbolic")
        .build();
    node_handle.add_css_class("dim-label");

    if !locked {
        let drag_source = gtk::DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE);
        {
            let id = sub_id;
            let source_idx = idx;
            drag_source.connect_prepare(move |_src, _x, _y| {
                Some(gdk::ContentProvider::for_value(
                    &format!("node_{id}_{source_idx}").to_value(),
                ))
            });
        }
        node_handle.add_controller(drag_source);

        let drop_target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        {
            let target_id = sub_id;
            let target_idx = idx;
            let s = sender.clone();
            drop_target.connect_drop(move |_target, value, _x, _y| {
                let prefix = format!("node_{target_id}_");
                if let Ok(val) = value.get::<String>()
                    && let Some(from_str) = val.strip_prefix(&prefix)
                    && let Ok(from_idx) = from_str.parse::<usize>()
                {
                    s.input(SubscriptionsMsg::DragDropNode(
                        target_id, from_idx, target_idx,
                    ));
                    return true;
                }
                false
            });
        }
        row.add_controller(drop_target);
    }
    row.add_prefix(&node_handle);

    let badge = gtk::Label::builder()
        .label(protocol)
        .css_classes(["caption", "accent"])
        .valign(gtk::Align::Center)
        .build();
    row.add_prefix(&badge);

    if active {
        let connected = gtk::Label::builder()
            .label("Connected")
            .css_classes(["caption", "success"])
            .valign(gtk::Align::Center)
            .tooltip_text("Currently connected node")
            .build();
        row.add_suffix(&connected);
    }

    if let Some(ms) = node.last_latency_ms {
        let latency_label = gtk::Label::builder()
            .label(format!("{ms}ms"))
            .valign(gtk::Align::Center)
            .build();
        latency_label.add_css_class("caption");
        const LATENCY_GOOD_MS: u64 = 200;
        const LATENCY_WARN_MS: u64 = 500;
        if ms < LATENCY_GOOD_MS {
            latency_label.add_css_class("success");
        } else if ms < LATENCY_WARN_MS {
            latency_label.add_css_class("warning");
        } else {
            latency_label.add_css_class("error");
        }
        row.add_suffix(&latency_label);
    }

    if let Some(ms) = node.last_real_delay_ms {
        let real_label = gtk::Label::builder()
            .label(format!("· {ms}ms"))
            .valign(gtk::Align::Center)
            .build();
        real_label.add_css_class("caption");
        const LATENCY_GOOD_MS: u64 = 200;
        const LATENCY_WARN_MS: u64 = 500;
        if ms < LATENCY_GOOD_MS {
            real_label.add_css_class("success");
        } else if ms < LATENCY_WARN_MS {
            real_label.add_css_class("warning");
        } else {
            real_label.add_css_class("error");
        }
        row.add_suffix(&real_label);
    }

    let move_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .valign(gtk::Align::Center)
        .build();

    let connect_btn = gtk::Button::builder()
        .icon_name("network-connect-symbolic")
        .has_frame(false)
        .tooltip_text("Connect")
        .sensitive(node.enabled && !locked)
        .build();
    connect_btn.add_css_class("flat");
    {
        let s = sender.clone();
        let node_id = node.id;
        connect_btn.connect_clicked(move |_| {
            s.input(SubscriptionsMsg::ConnectNode(sub_id, node_id));
        });
    }

    let up_btn = gtk::Button::builder()
        .icon_name("go-up-symbolic")
        .has_frame(false)
        .tooltip_text("Move Up")
        .sensitive(!locked)
        .build();
    up_btn.add_css_class("flat");
    {
        let s = sender.clone();
        up_btn.connect_clicked(move |_| {
            s.input(SubscriptionsMsg::MoveNode(sub_id, idx, Direction::Up));
        });
    }

    let down_btn = gtk::Button::builder()
        .icon_name("go-down-symbolic")
        .has_frame(false)
        .tooltip_text("Move Down")
        .sensitive(!locked)
        .build();
    down_btn.add_css_class("flat");
    {
        let s = sender.clone();
        down_btn.connect_clicked(move |_| {
            s.input(SubscriptionsMsg::MoveNode(sub_id, idx, Direction::Down));
        });
    }

    move_box.append(&connect_btn);
    move_box.append(&up_btn);
    move_box.append(&down_btn);
    row.add_suffix(&move_box);

    let node_toggle = gtk::Switch::builder()
        .active(node.enabled)
        .valign(gtk::Align::Center)
        .sensitive(!locked)
        .build();
    {
        let s = sender.clone();
        node_toggle.connect_active_notify(move |_| {
            s.input(SubscriptionsMsg::ToggleNode(sub_id, idx));
        });
    }
    row.add_suffix(&node_toggle);

    row
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let boundary = s
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= max)
            .last()
            .unwrap_or(0);
        format!("{}...", &s[..boundary])
    }
}

/// Node row subtitle: the address, plus the last known egress when one exists.
fn format_node_subtitle(address: &str, egress: Option<&EgressSample>) -> String {
    match egress {
        Some(sample) => format!("{address} · exit: {} · {}", sample.location, sample.address),
        None => address.to_string(),
    }
}

/// Inline hint shown next to a blocked verdict, so the rejected exit is
/// visible instead of inferable.
fn format_blocked_hint(egress: &EgressSample) -> String {
    format!("exit: {} · {}", egress.location, egress.address)
}

/// Badge styling for a verdict: available and blocked are colored, unknown is
/// styled distinct from both.
fn verdict_css_classes(verdict: AvailabilityVerdict) -> &'static [&'static str] {
    match verdict {
        AvailabilityVerdict::Available => &["caption", "success"],
        AvailabilityVerdict::Blocked => &["caption", "error"],
        AvailabilityVerdict::Unknown => &["caption", "dim-label"],
    }
}

/// Opens the availability results view: probed nodes against the built-in
/// services, one badge per verdict with its checked-at time, and the node's
/// last known egress next to blocked verdicts.
fn show_availability_dialog(
    sub: &Subscription,
    verdicts: &HashMap<(Uuid, Uuid), HashMap<String, VerdictRecord>>,
    egress_samples: &HashMap<(Uuid, Uuid), EgressSample>,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(format!("Availability — {}", sub.name))
        .body("Verdicts are measured through each node's own tunnel without touching the active connection")
        .build();
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();

    let services = builtin_service_checks();
    let mut rendered = 0usize;
    for node in &sub.nodes {
        let Some(node_verdicts) = verdicts.get(&(sub.id, node.id)) else {
            continue;
        };
        let group = adw::PreferencesGroup::builder()
            .title(node.node.remark().unwrap_or("Unnamed Node"))
            .build();
        let mut group_rows = 0usize;
        for service in services {
            let Some(record) = node_verdicts.get(service.id) else {
                continue;
            };
            let checked_at = record
                .checked_at
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string();
            let mut subtitle = format!("Checked: {checked_at}");
            if record.result == AvailabilityVerdict::Blocked
                && let Some(egress) = egress_samples.get(&(sub.id, node.id))
            {
                subtitle.push_str(&format!(" · {}", format_blocked_hint(egress)));
            }
            let row = adw::ActionRow::builder()
                .title(service.name)
                .subtitle(subtitle)
                .build();
            let badge = gtk::Label::builder()
                .label(record.result.label())
                .valign(gtk::Align::Center)
                .build();
            for class in verdict_css_classes(record.result) {
                badge.add_css_class(class);
            }
            row.add_suffix(&badge);
            group.add(&row);
            group_rows += 1;
            rendered += 1;
        }
        if group_rows > 0 {
            content.append(&group);
        }
    }
    if rendered == 0 {
        let empty = gtk::Label::builder()
            .label("No availability verdicts recorded yet. Run Check Availability first.")
            .css_classes(["dim-label"])
            .build();
        content.append(&empty);
    }

    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(200)
        .max_content_height(420)
        .propagate_natural_width(true)
        .child(&content)
        .build();
    dialog.set_extra_child(Some(&scroll));
    dialog.present(crate::active_window().as_ref());
}

fn show_add_dialog(sender: ComponentSender<SubscriptionsPage>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Add Subscription")
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add");
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let name_entry = adw::EntryRow::builder().title("Name").build();
    let url_entry = adw::EntryRow::builder().title("URL").build();

    let group = adw::PreferencesGroup::new();
    group.add(&name_entry);
    group.add(&url_entry);
    let file_entry = adw::EntryRow::builder().title("Local File Path").build();
    group.add(&file_entry);
    content.append(&group);

    let paste_label = gtk::Label::builder()
        .label("Or paste a config-bundle JSON subscription")
        .halign(gtk::Align::Start)
        .css_classes(["caption"])
        .build();
    content.append(&paste_label);

    let paste_buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
    let paste_view = gtk::TextView::builder()
        .buffer(&paste_buffer)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(6)
        .right_margin(6)
        .build();
    let paste_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(96)
        .max_content_height(160)
        .child(&paste_view)
        .css_classes(["card"])
        .build();
    content.append(&paste_scroll);

    dialog.set_extra_child(Some(&content));

    dialog.connect_response(None, move |_, response| {
        if response == "add" {
            let name = name_entry.text().to_string();
            let url = strip_v2raytun_import_prefix(url_entry.text().trim());
            let file_path = file_entry.text().to_string();
            let (start, end) = paste_buffer.bounds();
            let pasted_json = paste_buffer.text(&start, &end, false).to_string();
            if !name.trim().is_empty()
                && let Some(source) =
                    subscription_source_from_inputs(&url, file_path.trim(), pasted_json.trim())
            {
                sender.input(SubscriptionsMsg::AddSubscription(
                    name.trim().into(),
                    source,
                ));
            }
        }
    });

    dialog.present(crate::active_window().as_ref());
}

const V2RAYTUN_IMPORT_PREFIX: &str = "v2raytun://import/";

/// Strips a `v2raytun://import/` deep-link prefix and recovers the
/// underlying subscription URL, which the app encodes URL-then-base64.
fn strip_v2raytun_import_prefix(input: &str) -> String {
    let Some(rest) = input.strip_prefix(V2RAYTUN_IMPORT_PREFIX) else {
        return input.to_string();
    };
    let decoded = percent_encoding::percent_decode_str(rest)
        .decode_utf8()
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| rest.to_string());
    base64::engine::general_purpose::STANDARD
        .decode(&decoded)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&decoded))
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or(decoded)
}

#[derive(Debug, Clone)]
pub(crate) enum SubscriptionSourceInput {
    Url(String),
    File(String),
    Paste(String),
}

pub(crate) fn subscription_source_from_inputs(
    url: &str,
    file_path: &str,
    pasted_json: &str,
) -> Option<SubscriptionSourceInput> {
    let url = url.trim();
    let file_path = file_path.trim();
    let pasted_json = pasted_json.trim();
    match (url.is_empty(), file_path.is_empty(), pasted_json.is_empty()) {
        (false, true, true) => Some(SubscriptionSourceInput::Url(url.into())),
        (true, false, true) => Some(SubscriptionSourceInput::File(file_path.into())),
        (true, true, false) => Some(SubscriptionSourceInput::Paste(pasted_json.into())),
        _ => None,
    }
}

fn write_subscription_blob(
    paths: &AppPaths,
    content: &str,
) -> Result<std::path::PathBuf, PersistenceError> {
    paths.ensure_dirs()?;
    let path = paths
        .subscription_blobs_dir()
        .join(format!("{}.json", Uuid::new_v4()));
    std::fs::write(&path, content.as_bytes()).map_err(PersistenceError::Io)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(PersistenceError::Io)?;
    }

    Ok(path)
}

fn show_rename_dialog(id: Uuid, current_name: &str, sender: ComponentSender<SubscriptionsPage>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Rename Subscription")
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("rename", "Rename");
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let name_entry = adw::EntryRow::builder()
        .title("Name")
        .text(current_name)
        .build();

    let group = adw::PreferencesGroup::new();
    group.add(&name_entry);
    content.append(&group);

    dialog.set_extra_child(Some(&content));

    dialog.connect_response(None, move |_, response| {
        if response == "rename" {
            let new_name = name_entry.text().to_string();
            if !new_name.trim().is_empty() {
                sender.input(SubscriptionsMsg::RenameSubscription(
                    id,
                    new_name.trim().into(),
                ));
            }
        }
    });

    dialog.present(crate::active_window().as_ref());
}

fn show_delete_dialog(id: Uuid, sender: ComponentSender<SubscriptionsPage>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Delete Subscription")
        .body("Are you sure you want to delete this subscription?")
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            sender.input(SubscriptionsMsg::DeleteSubscription(id));
        }
    });

    dialog.present(crate::active_window().as_ref());
}

#[cfg(test)]
mod availability_view_tests {
    use super::*;
    use chrono::Utc;

    fn egress(loc: &str, addr: &str) -> EgressSample {
        EgressSample {
            address: addr.to_string(),
            location: loc.to_string(),
            checked_at: Utc::now(),
        }
    }

    #[test]
    fn node_subtitle_shows_exit_only_when_known() {
        assert_eq!(
            format_node_subtitle("a.example.com:443", None),
            "a.example.com:443"
        );
        let sample = egress("AT", "2001:db8::1");
        assert_eq!(
            format_node_subtitle("a.example.com:443", Some(&sample)),
            "a.example.com:443 · exit: AT · 2001:db8::1"
        );
    }

    #[test]
    fn blocked_hint_formats_location_and_address() {
        assert_eq!(
            format_blocked_hint(&egress("DE", "203.0.113.7")),
            "exit: DE · 203.0.113.7"
        );
    }

    #[test]
    fn verdict_badge_styles_distinguish_the_three_states() {
        let available = verdict_css_classes(AvailabilityVerdict::Available);
        let blocked = verdict_css_classes(AvailabilityVerdict::Blocked);
        let unknown = verdict_css_classes(AvailabilityVerdict::Unknown);
        assert!(available.contains(&"success"));
        assert!(blocked.contains(&"error"));
        // Unknown must not share the colored styling of the other two.
        assert!(!unknown.contains(&"success") && !unknown.contains(&"error"));
        assert!(unknown.contains(&"dim-label"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use v2ray_rs_core::models::{ProxyNode, SubscriptionNode, VlessConfig};
    use v2ray_rs_subscription::ParseFailure;

    fn create_test_subscription(
        name: &str,
        enabled: bool,
        nodes: Vec<SubscriptionNode>,
    ) -> Subscription {
        Subscription {
            id: Uuid::new_v4(),
            name: name.to_string(),
            source: v2ray_rs_core::models::SubscriptionSource::Url {
                url: "https://example.com".to_string(),
            },
            nodes,
            last_updated: None,
            auto_update_interval_secs: None,
            enabled,
            imported_profile: None,
            use_imported_profile: true,
        }
    }

    fn create_test_node(address: &str, port: u16, enabled: bool) -> SubscriptionNode {
        let addr: SocketAddr = format!("{address}:{port}").parse().unwrap();
        let mut node = SubscriptionNode::new(ProxyNode::Vless(VlessConfig {
            address: addr.ip().to_string(),
            port: addr.port(),
            uuid: Uuid::new_v4().to_string(),
            encryption: None,
            flow: None,
            transport: Default::default(),
            tls: None,
            remark: None,
        }));
        node.enabled = enabled;
        node
    }

    #[test]
    fn subscription_failure_toasts_once_per_streak() {
        let mut streaks = HashMap::new();
        let id = Uuid::new_v4();
        let fail: AutoUpdateRefreshResult = Err("timeout".into());
        let ok: AutoUpdateRefreshResult = Ok((
            create_test_subscription("Corp", true, vec![]),
            UpdateResult {
                added: 0,
                removed: 0,
                unchanged: 0,
                parse_failures: Vec::new(),
                profile: None,
            },
        ));

        assert_eq!(
            auto_update_outcome_toast(&mut streaks, id, "Corp", &fail),
            Some("Auto-update failed for Corp: timeout".to_string())
        );
        assert_eq!(
            auto_update_outcome_toast(&mut streaks, id, "Corp", &fail),
            None
        );
        assert_eq!(
            auto_update_outcome_toast(&mut streaks, id, "Corp", &ok),
            None
        );

        let other = Uuid::new_v4();
        assert_eq!(
            auto_update_outcome_toast(&mut streaks, other, "Other", &ok),
            None
        );
        assert_eq!(
            auto_update_outcome_toast(&mut streaks, other, "Other", &fail),
            Some("Auto-update failed for Other: timeout".to_string())
        );

        assert_eq!(
            auto_update_outcome_toast(&mut streaks, id, "Corp", &fail),
            Some("Auto-update failed for Corp: timeout".to_string())
        );
    }

    fn parse_failure(uri: &str) -> ParseFailure {
        ParseFailure {
            uri: uri.to_string(),
            error: "unsupported scheme".to_string(),
        }
    }

    #[test]
    fn update_summary_redacts_credentials() {
        let result = UpdateResult {
            added: 1,
            removed: 0,
            unchanged: 2,
            parse_failures: vec![parse_failure(
                "trojan://secret-password@example.com:443?allowInsecure=1#node-1",
            )],
            profile: None,
        };

        let summary = format_update_summary("Corp", "Updated", &result);
        assert!(summary.contains("example.com:443"), "{summary}");
        assert!(!summary.contains("secret-password"), "{summary}");
        assert!(!summary.contains("allowInsecure"), "{summary}");
        assert!(!summary.contains("node-1"), "{summary}");
        assert!(summary.contains("1 invalid URI(s)"), "{summary}");
        assert!(summary.contains("line 1:"), "{summary}");
    }

    #[test]
    fn subscription_error_redacts_credentials() {
        let error = SubscriptionError::Update(UpdateError::InvalidContent {
            failures: vec![parse_failure(
                "vless://11111111-2222-3333-4444-555555555555@203.0.113.7:8443?type=ws",
            )],
        });

        let message = format_subscription_error(&error);
        assert!(message.contains("203.0.113.7:8443"), "{message}");
        assert!(
            !message.contains("11111111-2222-3333-4444-555555555555"),
            "{message}"
        );
        assert!(!message.contains("type=ws"), "{message}");

        let raw = parse_failure("not even a uri");
        let result = UpdateResult {
            added: 0,
            removed: 0,
            unchanged: 0,
            parse_failures: vec![raw],
            profile: None,
        };
        let summary = format_update_summary("Corp", "Updated", &result);
        assert!(!summary.contains("not even a uri"), "{summary}");
        assert!(summary.contains("unrecognized entry"), "{summary}");
    }

    #[test]
    fn enabled_subscription_with_enabled_nodes_eligible() {
        let sub = create_test_subscription(
            "Test Sub",
            true,
            vec![create_test_node("127.0.0.1", 8080, true)],
        );
        let testing_latency = HashSet::new();

        let eligible = subscriptions_eligible_for_latency_test(&[sub], &testing_latency, false);

        assert_eq!(eligible.len(), 1);
    }

    #[test]
    fn disabled_subscription_not_eligible() {
        let sub = create_test_subscription(
            "Test Sub",
            false,
            vec![create_test_node("127.0.0.1", 8080, true)],
        );
        let testing_latency = HashSet::new();

        let eligible = subscriptions_eligible_for_latency_test(&[sub], &testing_latency, false);

        assert!(eligible.is_empty());
    }

    #[test]
    fn subscription_with_no_enabled_nodes_not_eligible() {
        let sub = create_test_subscription(
            "Test Sub",
            true,
            vec![
                create_test_node("127.0.0.1", 8080, false),
                create_test_node("127.0.0.2", 8081, false),
            ],
        );
        let testing_latency = HashSet::new();

        let eligible = subscriptions_eligible_for_latency_test(&[sub], &testing_latency, false);

        assert!(eligible.is_empty());
    }

    #[test]
    fn subscription_already_testing_not_eligible() {
        let sub = create_test_subscription(
            "Test Sub",
            true,
            vec![create_test_node("127.0.0.1", 8080, true)],
        );
        let mut testing_latency = HashSet::new();
        testing_latency.insert(sub.id);

        let eligible = subscriptions_eligible_for_latency_test(&[sub], &testing_latency, false);

        assert!(eligible.is_empty());
    }

    #[test]
    fn mixed_subscriptions_correct_subset_selected() {
        let sub1 = create_test_subscription(
            "Enabled with nodes",
            true,
            vec![
                create_test_node("127.0.0.1", 8080, true),
                create_test_node("127.0.0.2", 8081, true),
            ],
        );
        let sub2 = create_test_subscription(
            "Disabled subscription",
            false,
            vec![create_test_node("127.0.0.3", 8082, true)],
        );
        let sub3 = create_test_subscription(
            "Enabled but no enabled nodes",
            true,
            vec![create_test_node("127.0.0.4", 8083, false)],
        );
        let sub4 = create_test_subscription(
            "Already testing",
            true,
            vec![create_test_node("127.0.0.5", 8084, true)],
        );
        let sub1_id = sub1.id;
        let mut testing_latency = HashSet::new();
        testing_latency.insert(sub4.id);

        let eligible = subscriptions_eligible_for_latency_test(
            &[sub1, sub2, sub3, sub4],
            &testing_latency,
            false,
        );

        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0], sub1_id);
    }

    #[test]
    fn tun_active_suppresses_all_latency_probes() {
        let sub = create_test_subscription(
            "Test Sub",
            true,
            vec![create_test_node("127.0.0.1", 8080, true)],
        );
        let testing_latency = HashSet::new();

        let eligible = subscriptions_eligible_for_latency_test(&[sub], &testing_latency, true);

        assert!(eligible.is_empty());
    }

    #[test]
    fn tun_inactive_resumes_latency_probes() {
        let sub = create_test_subscription(
            "Test Sub",
            true,
            vec![create_test_node("127.0.0.1", 8080, true)],
        );
        let sub_id = sub.id;
        let testing_latency = HashSet::new();

        let eligible = subscriptions_eligible_for_latency_test(&[sub], &testing_latency, false);

        assert_eq!(eligible, vec![sub_id]);
    }

    #[test]
    fn merge_refreshed_subscription_preserves_local_runtime_state() {
        let mut current = create_test_subscription(
            "Renamed subscription",
            true,
            vec![create_test_node("127.0.0.1", 8080, false)],
        );
        current.auto_update_interval_secs = Some(7200);
        current.nodes[0].last_latency_ms = Some(37);

        let refreshed = Subscription {
            id: current.id,
            name: "Original subscription".into(),
            source: current.source.clone(),
            nodes: vec![
                create_test_node("127.0.0.1", 8080, true),
                create_test_node("127.0.0.2", 8081, true),
            ],
            last_updated: Some(chrono::Utc::now()),
            auto_update_interval_secs: None,
            enabled: false,
            imported_profile: None,
            use_imported_profile: true,
        };

        let merged = merge_refreshed_subscription(&current, refreshed.clone());

        assert_eq!(merged.name, current.name);
        assert_eq!(merged.source, current.source);
        assert_eq!(
            merged.auto_update_interval_secs,
            current.auto_update_interval_secs
        );
        assert_eq!(merged.enabled, current.enabled);
        assert_eq!(merged.last_updated, refreshed.last_updated);
        assert_eq!(merged.nodes.len(), 2);
        assert_eq!(merged.nodes[0].id, current.nodes[0].id);
        assert!(!merged.nodes[0].enabled);
        assert_eq!(merged.nodes[0].last_latency_ms, Some(37));
        assert_eq!(merged.nodes[1].node.address(), "127.0.0.2");
        assert!(merged.nodes[1].enabled);
    }

    #[test]
    fn merge_refreshed_subscription_preserves_disabled_profile_toggle() {
        let mut current = create_test_subscription(
            "Provider sub",
            true,
            vec![create_test_node("127.0.0.1", 8080, true)],
        );
        current.use_imported_profile = false;
        current.imported_profile = Some(v2ray_rs_core::models::ImportedProfile {
            rules: vec![],
            dns: None,
            skipped: vec![],
            imported_at: chrono::Utc::now(),
        });

        let refreshed = Subscription {
            id: current.id,
            name: current.name.clone(),
            source: current.source.clone(),
            nodes: vec![create_test_node("127.0.0.1", 8080, true)],
            last_updated: Some(chrono::Utc::now()),
            auto_update_interval_secs: None,
            enabled: true,
            imported_profile: Some(v2ray_rs_core::models::ImportedProfile {
                rules: vec![],
                dns: None,
                skipped: vec![],
                imported_at: chrono::Utc::now(),
            }),
            use_imported_profile: true,
        };

        let merged = merge_refreshed_subscription(&current, refreshed);

        assert!(
            !merged.use_imported_profile,
            "refresh must not silently re-enable a disabled profile"
        );
        assert!(merged.imported_profile.is_some());
    }

    #[test]
    fn merge_refreshed_subscription_keeps_stale_profile_when_refresh_has_none() {
        let mut current = create_test_subscription(
            "Provider sub",
            true,
            vec![create_test_node("127.0.0.1", 8080, true)],
        );
        current.imported_profile = Some(v2ray_rs_core::models::ImportedProfile {
            rules: vec![],
            dns: None,
            skipped: vec![],
            imported_at: chrono::Utc::now(),
        });

        let refreshed = Subscription {
            id: current.id,
            name: current.name.clone(),
            source: current.source.clone(),
            nodes: vec![create_test_node("127.0.0.1", 8080, true)],
            last_updated: Some(chrono::Utc::now()),
            auto_update_interval_secs: None,
            enabled: true,
            imported_profile: None,
            use_imported_profile: true,
        };

        let merged = merge_refreshed_subscription(&current, refreshed);

        assert!(
            merged.imported_profile.is_some(),
            "a refresh that lost the bundle body must keep the stale profile"
        );
    }

    #[test]
    #[cfg(unix)]
    fn write_subscription_blob_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_profile_in(v2ray_rs_core::profile::AppProfile::Test, tmp.path());

        let blob_path = write_subscription_blob(&paths, "[]").unwrap();

        let mode = std::fs::metadata(&blob_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "subscription blob holds live credentials and must not be group/world readable"
        );
    }

    #[test]
    fn latency_results_follow_snapshot_ids_after_reorder() {
        let mut sub = create_test_subscription(
            "Reordered",
            true,
            vec![
                create_test_node("10.0.0.1", 1001, true),
                create_test_node("10.0.0.2", 1002, true),
                create_test_node("10.0.0.3", 1003, true),
            ],
        );
        let snapshot_ids: Vec<Uuid> = sub.nodes.iter().map(|n| n.id).collect();
        let results = vec![Some(30), None, Some(70)];

        sub.nodes.swap(0, 2);
        apply_latency_by_ids(&mut sub, &snapshot_ids, &results);

        let latency = |idx: usize| sub.nodes[idx].last_latency_ms;
        assert_eq!(
            latency(2),
            Some(30),
            "result must follow the node id, not position"
        );
        assert_eq!(latency(1), None);
        assert_eq!(latency(0), Some(70));
    }

    #[test]
    fn latency_results_for_removed_nodes_are_dropped() {
        let mut sub = create_test_subscription(
            "Shrunk",
            true,
            vec![
                create_test_node("10.0.0.1", 1001, true),
                create_test_node("10.0.0.2", 1002, true),
            ],
        );
        let removed_id = sub.nodes[1].id;
        let snapshot_ids = vec![sub.nodes[0].id, removed_id];
        let results = vec![Some(11), Some(22)];

        sub.nodes.remove(1);
        apply_latency_by_ids(&mut sub, &snapshot_ids, &results);

        assert_eq!(sub.nodes[0].last_latency_ms, Some(11));
        assert!(
            sub.nodes
                .iter()
                .all(|n| n.id == snapshot_ids[0] || n.last_latency_ms.is_none())
        );
    }

    #[test]
    fn real_delay_results_follow_snapshot_ids_after_reorder() {
        let mut sub = create_test_subscription(
            "Reordered delays",
            true,
            vec![
                create_test_node("10.0.0.1", 1001, true),
                create_test_node("10.0.0.2", 1002, true),
            ],
        );
        let snapshot_ids: Vec<Uuid> = sub.nodes.iter().map(|n| n.id).collect();
        let results = vec![Some(120), None];

        sub.nodes.reverse();
        apply_real_delay_by_ids(&mut sub, &snapshot_ids, &results);

        assert_eq!(sub.nodes[1].last_real_delay_ms, Some(120));
        assert_eq!(sub.nodes[0].last_real_delay_ms, None);
    }

    #[test]
    fn duplicate_refresh_is_suppressed_until_completion_marker_clears_it() {
        let id = Uuid::new_v4();
        let mut refreshing = HashSet::new();

        assert!(should_start_refresh(&refreshing, id));
        refreshing.insert(id);
        assert!(!should_start_refresh(&refreshing, id));
        refreshing.remove(&id);
        assert!(should_start_refresh(&refreshing, id));
    }

    #[test]
    fn persist_failure_maps_to_save_notice() {
        let err = PersistenceError::NoDirs;
        assert_eq!(
            subscription_persist_notice(&err),
            "Failed to save subscriptions: failed to determine XDG directories"
        );
    }
}
