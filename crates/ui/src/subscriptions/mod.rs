use adw::prelude::*;
use base64::Engine;
use gtk::gdk;
use relm4::adw;
use relm4::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

use v2ray_rs_core::geo_probe::{AvailabilityVerdict, ServiceCheck, builtin_service_checks};
use v2ray_rs_core::models::{
    AppSettings, BackendType, ConnectionNodeRef, EgressSample, ProxyNode, RealDelayCapability,
    RealDelaySettings, Subscription, SubscriptionSource, VerdictRecord,
};
use v2ray_rs_core::persistence::{AppPaths, PersistenceError};
use v2ray_rs_core::runtime_snapshot::subscriptions_runtime_state_eq;
use v2ray_rs_subscription::{
    MAX_SUBSCRIPTION_NODES, MAX_SUBSCRIPTION_SIZE, RealDelayReport, SubscriptionError,
    SubscriptionImportOutcome, SubscriptionService, UpdateError, UpdateResult,
    parse_json_subscription, reconcile_nodes, same_connection, subscription_url_is_public,
};

use crate::failure_streak::FailureStreak;
use crate::i18n::tr;
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
    writes: WriteTracker,
    imports: ImportGeneration,
    load_generation: u64,
    loading: bool,
    reset_pending: bool,
    active_nodes_reported: bool,
    testing_real_delay: HashMap<Uuid, u64>,
    real_delay_run_token: u64,
    testing_availability: HashMap<Uuid, u64>,
    availability_run_token: u64,
    egress_samples: HashMap<(Uuid, Uuid), EgressSample>,
    verdicts: HashMap<(Uuid, Uuid), HashMap<String, VerdictRecord>>,
    expanded_subs: HashSet<Uuid>,
    sub_expanders: HashMap<Uuid, adw::ExpanderRow>,
    node_widgets: HashMap<(Uuid, Uuid), NodeWidgets>,
    active_node: Option<(Uuid, Uuid)>,
    tun_active: bool,
    backend_type: BackendType,
    binary_path: Option<std::path::PathBuf>,
    real_delay_settings: RealDelaySettings,
    real_delay_capability: RealDelayCapability,
    paths: AppPaths,
}

mod dialogs;
mod probes;
mod rows;
use dialogs::*;
pub(crate) use dialogs::{strip_v2raytun_import_prefix, subscription_source_from_inputs};
use probes::*;
use rows::*;

enum RenderHint {
    Full,
    NodeState,
    NodeToggle(Uuid, usize),
    SubscriptionToggle(Uuid),
    SubscriptionRename(Uuid),
}

/// Widgets owned by one rendered subscription node, indexed by its stable
/// `NodeId` so updates never walk private `AdwExpanderRow` children.
struct NodeWidgets {
    row: adw::ActionRow,
    toggle: gtk::Switch,
    address: String,
    latency: gtk::Label,
    real_delay: gtk::Label,
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
    SubscriptionsPersisted,
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
    AddDone {
        generation: u64,
        outcome: SubscriptionImportOutcome,
    },
    RefreshDone(Subscription, UpdateResult),
    LatencyResult(Uuid, Vec<ProbeTarget>, Vec<Option<u64>>),
    RealDelayResult(Uuid, u64, Vec<ProbeTarget>, RealDelayReport),
    AvailabilityResult(Uuid, u64, Vec<ProbeTarget>, RealDelayReport),
    RefreshFailed(Uuid, SubscriptionError),
    AddFailed {
        generation: u64,
        error: SubscriptionError,
    },
    AutoUpdateDone(AutoUpdateRefreshBatch),
    AutoUpdateTick(u64),
    BackgroundLatencyTick,
    InitialLoaded(InitialSubscriptions),
    SubscriptionsSaved(SubscriptionSaveOutcome),
    ResetSaved {
        revision: u64,
        result: Result<(), PersistenceError>,
    },
    SnapshotSaved(Result<(), PersistenceError>),
}

#[derive(Debug)]
pub(crate) struct InitialSubscriptions {
    generation: u64,
    result: Result<
        (
            Vec<Subscription>,
            Option<Arc<v2ray_rs_core::resolve::LatencySnapshot>>,
        ),
        PersistenceError,
    >,
}

#[derive(Debug)]
pub(crate) struct SubscriptionSaveOutcome {
    revision: u64,
    runtime: bool,
    result: Result<Arc<Vec<Subscription>>, PersistenceError>,
}

fn writer_stopped_message() -> String {
    "workspace writer stopped".to_string()
}

/// Orders subscription writes on the shared workspace writer and classifies
/// their receipts. Runtime outputs follow a durable receipt, never enqueueing.
#[derive(Default)]
struct WriteTracker {
    submitted: u64,
    committed: u64,
    pending: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReceiptStatus {
    durable: bool,
    latest: bool,
}

impl WriteTracker {
    fn submit(&mut self) -> u64 {
        self.submitted = self.submitted.wrapping_add(1);
        self.pending = self.pending.saturating_add(1);
        self.submitted
    }

    fn complete(&mut self, revision: u64, durable: bool) -> ReceiptStatus {
        self.pending = self.pending.saturating_sub(1);
        if durable && revision > self.committed {
            self.committed = revision;
        }
        ReceiptStatus {
            durable,
            latest: revision == self.submitted,
        }
    }
}

/// Bumped by a reset so an import that finishes later is discarded instead of
/// resurrecting a deleted subscription.
#[derive(Default)]
struct ImportGeneration(u64);

impl ImportGeneration {
    fn current(&self) -> u64 {
        self.0
    }

    fn invalidate(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }

    fn accepts(&self, generation: u64) -> bool {
        generation == self.0
    }
}

/// One ordered subscription collection edit. The same descriptor applies to
/// the local list and, inside the writer transaction, to the latest committed
/// list, so both stay in FIFO order without a second writer.
#[derive(Clone)]
enum SubscriptionEdit {
    ToggleEnabled(Uuid),
    ToggleUseImportedProfile(Uuid),
    ToggleNodeEnabled {
        subscription: Uuid,
        index: usize,
    },
    Rename {
        subscription: Uuid,
        name: String,
    },
    MoveSubscription {
        subscription: Uuid,
        direction: Direction,
    },
    MoveNode {
        subscription: Uuid,
        index: usize,
        direction: Direction,
    },
    Delete(Uuid),
    SortByLatency(Uuid),
    SortByRealDelay(Uuid),
    SetNodesEnabled {
        subscription: Uuid,
        enabled: bool,
    },
    DragSubscription {
        from: usize,
        to: usize,
    },
    DragNode {
        subscription: Uuid,
        from: usize,
        to: usize,
    },
    Push(Subscription),
    Replace(Subscription),
}

impl SubscriptionEdit {
    fn apply(&self, subscriptions: &mut Vec<Subscription>) -> bool {
        match self {
            Self::ToggleEnabled(id) => {
                let Some(subscription) = subscriptions.iter_mut().find(|s| s.id == *id) else {
                    return false;
                };
                subscription.enabled = !subscription.enabled;
                true
            }
            Self::ToggleUseImportedProfile(id) => {
                let Some(subscription) = subscriptions.iter_mut().find(|s| s.id == *id) else {
                    return false;
                };
                subscription.use_imported_profile = !subscription.use_imported_profile;
                true
            }
            Self::ToggleNodeEnabled {
                subscription,
                index,
            } => {
                let Some(node) = subscriptions
                    .iter_mut()
                    .find(|s| s.id == *subscription)
                    .and_then(|s| s.nodes.get_mut(*index))
                else {
                    return false;
                };
                node.enabled = !node.enabled;
                true
            }
            Self::Rename { subscription, name } => {
                let Some(subscription) = subscriptions.iter_mut().find(|s| s.id == *subscription)
                else {
                    return false;
                };
                subscription.name = name.clone();
                true
            }
            Self::MoveSubscription {
                subscription,
                direction,
            } => {
                let Some(position) = subscriptions.iter().position(|s| s.id == *subscription)
                else {
                    return false;
                };
                let target = match direction {
                    Direction::Up if position > 0 => position - 1,
                    Direction::Down if position + 1 < subscriptions.len() => position + 1,
                    _ => position,
                };
                if target == position {
                    return false;
                }
                subscriptions.swap(position, target);
                true
            }
            Self::MoveNode {
                subscription,
                index,
                direction,
            } => {
                let (index, direction) = (*index, *direction);
                let Some(subscription) = subscriptions.iter_mut().find(|s| s.id == *subscription)
                else {
                    return false;
                };
                let target = match direction {
                    Direction::Up if index > 0 => index - 1,
                    Direction::Down if index + 1 < subscription.nodes.len() => index + 1,
                    _ => index,
                };
                if target == index {
                    return false;
                }
                subscription.nodes.swap(index, target);
                true
            }
            Self::Delete(id) => {
                let before = subscriptions.len();
                subscriptions.retain(|s| s.id != *id);
                subscriptions.len() != before
            }
            Self::SortByLatency(id) => {
                let Some(subscription) = subscriptions.iter_mut().find(|s| s.id == *id) else {
                    return false;
                };
                subscription.nodes.sort_by(|a, b| {
                    a.last_latency_ms
                        .unwrap_or(u64::MAX)
                        .cmp(&b.last_latency_ms.unwrap_or(u64::MAX))
                });
                true
            }
            Self::SortByRealDelay(id) => {
                let Some(subscription) = subscriptions.iter_mut().find(|s| s.id == *id) else {
                    return false;
                };
                subscription.nodes.sort_by(|a, b| {
                    a.last_real_delay_ms
                        .unwrap_or(u64::MAX)
                        .cmp(&b.last_real_delay_ms.unwrap_or(u64::MAX))
                });
                true
            }
            Self::SetNodesEnabled {
                subscription,
                enabled,
            } => {
                let Some(subscription) = subscriptions.iter_mut().find(|s| s.id == *subscription)
                else {
                    return false;
                };
                for node in &mut subscription.nodes {
                    node.enabled = *enabled;
                }
                true
            }
            Self::DragSubscription { from, to } => {
                let (from, to) = (*from, *to);
                if from == to || from >= subscriptions.len() || to >= subscriptions.len() {
                    return false;
                }
                let subscription = subscriptions.remove(from);
                subscriptions.insert(to, subscription);
                true
            }
            Self::DragNode {
                subscription,
                from,
                to,
            } => {
                let (from, to) = (*from, *to);
                let Some(subscription) = subscriptions.iter_mut().find(|s| s.id == *subscription)
                else {
                    return false;
                };
                if from == to || from >= subscription.nodes.len() || to >= subscription.nodes.len()
                {
                    return false;
                }
                let node = subscription.nodes.remove(from);
                subscription.nodes.insert(to, node);
                true
            }
            Self::Push(subscription) => {
                subscriptions.push(subscription.clone());
                true
            }
            Self::Replace(subscription) => {
                let Some(existing) = subscriptions.iter_mut().find(|s| s.id == subscription.id)
                else {
                    return false;
                };
                *existing = subscription.clone();
                true
            }
        }
    }
}

type EgressSampleMap = HashMap<(Uuid, Uuid), EgressSample>;
type VerdictMap = HashMap<(Uuid, Uuid), HashMap<String, VerdictRecord>>;

/// Rebinds persisted latency samples onto freshly loaded subscriptions.
fn hydrate_latency_state(
    subscriptions: &mut [Subscription],
    snapshot: &v2ray_rs_core::resolve::LatencySnapshot,
) -> (EgressSampleMap, VerdictMap) {
    let mut egress_samples = HashMap::new();
    let mut verdicts = HashMap::new();
    for subscription in subscriptions.iter_mut() {
        for node in &mut subscription.nodes {
            let node_ref = ConnectionNodeRef::Subscription {
                subscription_id: subscription.id,
                node_id: node.id,
            };
            if let Some(sample) = snapshot.get(node_ref) {
                node.last_latency_ms = sample.latency_ms;
                node.last_real_delay_ms = sample.real_delay_ms;
                if let Some(egress) = sample.egress.clone() {
                    egress_samples.insert((subscription.id, node.id), egress);
                }
                if !sample.verdicts.is_empty() {
                    verdicts.insert((subscription.id, node.id), sample.verdicts.clone());
                }
            }
        }
    }
    (egress_samples, verdicts)
}

fn subscription_persist_notice(err: &PersistenceError) -> String {
    format!("Failed to save subscriptions: {err}")
}

fn should_start_refresh(refreshing: &HashSet<Uuid>, id: Uuid) -> bool {
    !refreshing.contains(&id)
}

/// Node id plus the connection captured when its probe launched. Results
/// measured against a different connection are dropped instead of attached.
#[derive(Debug)]
pub(crate) struct ProbeTarget {
    node_id: Uuid,
    connection: ProxyNode,
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
                            .unwrap_or(&tr("Failed to load subscriptions")),
                    },

                    gtk::Button {
                        set_label: &tr("Reset Data"),
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
                    set_tooltip_text: Some(&tr("Add Subscription")),
                    add_css_class: "flat",
                    #[watch]
                    set_sensitive: model.load_error.is_none() && !model.loading,
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
        let subscriptions = Vec::new();
        let load_error = None;
        let egress_samples = HashMap::new();
        let verdicts = HashMap::new();

        let list_container = gtk::ListBox::builder()
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["boxed-list"])
            .selection_mode(gtk::SelectionMode::None)
            .build();

        let mut model = SubscriptionsPage {
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
            writes: WriteTracker::default(),
            imports: ImportGeneration::default(),
            load_generation: 0,
            loading: true,
            reset_pending: false,
            active_nodes_reported: false,
            testing_real_delay: HashMap::new(),
            real_delay_run_token: 0,
            testing_availability: HashMap::new(),
            availability_run_token: 0,
            egress_samples,
            verdicts,
            expanded_subs: HashSet::new(),
            sub_expanders: HashMap::new(),
            node_widgets: HashMap::new(),
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
            &mut model.sub_expanders,
            &mut model.node_widgets,
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

        let generation = model.load_generation;
        let load_store = store.clone();
        sender.oneshot_command(async move {
            let receipt = load_store.enqueue_warm();
            let result = match receipt.await {
                Ok(Ok(warm)) => match (warm.subscriptions, warm.latency) {
                    (Ok(subscriptions), latency) => Ok(((*subscriptions).clone(), latency.ok())),
                    (Err(err), _) => Err(err),
                },
                Ok(Err(err)) => Err(err),
                Err(_) => Err(PersistenceError::CorruptConfig(writer_stopped_message())),
            };
            SubscriptionsCmdOutput::InitialLoaded(InitialSubscriptions { generation, result })
        });

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
        if self.loading
            || (self.load_error.is_some()
                && !matches!(
                    msg,
                    SubscriptionsMsg::ResetStorage
                        | SubscriptionsMsg::SetActiveNode(_)
                        | SubscriptionsMsg::SetTunActive(_)
                        | SubscriptionsMsg::SyncSettings { .. }
                ))
        {
            return;
        }

        if self.reset_pending
            && !matches!(
                msg,
                SubscriptionsMsg::ResetStorage
                    | SubscriptionsMsg::SetActiveNode(_)
                    | SubscriptionsMsg::SetTunActive(_)
                    | SubscriptionsMsg::SyncSettings { .. }
                    | SubscriptionsMsg::ExpanderToggled(..)
                    | SubscriptionsMsg::ConnectNode(..)
            )
        {
            return;
        }

        let mut render_hint = RenderHint::Full;

        match msg {
            SubscriptionsMsg::ToggleSubscription(id) => {
                if self.apply_edits(&sender, true, vec![SubscriptionEdit::ToggleEnabled(id)]) {
                    render_hint = RenderHint::SubscriptionToggle(id);
                }
            }
            SubscriptionsMsg::ToggleUseImportedProfile(id) => {
                self.apply_edits(
                    &sender,
                    true,
                    vec![SubscriptionEdit::ToggleUseImportedProfile(id)],
                );
            }
            SubscriptionsMsg::ToggleNode(sub_id, idx) => {
                if self.apply_edits(
                    &sender,
                    true,
                    vec![SubscriptionEdit::ToggleNodeEnabled {
                        subscription: sub_id,
                        index: idx,
                    }],
                ) {
                    render_hint = RenderHint::NodeToggle(sub_id, idx);
                }
            }
            SubscriptionsMsg::RenameSubscription(id, new_name) => {
                if self.apply_edits(
                    &sender,
                    false,
                    vec![SubscriptionEdit::Rename {
                        subscription: id,
                        name: new_name,
                    }],
                ) {
                    render_hint = RenderHint::SubscriptionRename(id);
                }
            }
            SubscriptionsMsg::MoveSubscription(id, direction) => {
                self.apply_edits(
                    &sender,
                    true,
                    vec![SubscriptionEdit::MoveSubscription {
                        subscription: id,
                        direction,
                    }],
                );
            }
            SubscriptionsMsg::MoveNode(sub_id, idx, direction) => {
                self.apply_edits(
                    &sender,
                    true,
                    vec![SubscriptionEdit::MoveNode {
                        subscription: sub_id,
                        index: idx,
                        direction,
                    }],
                );
            }
            SubscriptionsMsg::DeleteSubscription(id) => {
                if self.apply_edits(&sender, true, vec![SubscriptionEdit::Delete(id)]) {
                    self.expanded_subs.remove(&id);
                }
            }
            SubscriptionsMsg::AddSubscription(name, source_input) => {
                let source = match source_input {
                    SubscriptionSourceInput::Url(url) => SubscriptionSource::Url { url },
                    SubscriptionSourceInput::File(path) => SubscriptionSource::File { path },
                    SubscriptionSourceInput::Paste(json) => {
                        match write_pasted_subscription_blob(&self.paths, &json) {
                            Ok(path) => SubscriptionSource::File {
                                path: path.to_string_lossy().into_owned(),
                            },
                            Err(err) => {
                                let _ = sender.output(SubscriptionsOutput::Notice(err));
                                return;
                            }
                        }
                    }
                };
                let svc = self.service.clone();
                let generation = self.imports.current();
                sender.oneshot_command(async move {
                    match svc.add_and_fetch(name, source).await {
                        Ok(outcome) => SubscriptionsCmdOutput::AddDone {
                            generation,
                            outcome,
                        },
                        Err(error) => SubscriptionsCmdOutput::AddFailed { generation, error },
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
                    let (targets, results) = run_latency_probe(nodes).await;
                    SubscriptionsCmdOutput::LatencyResult(id, targets, results)
                });
            }
            SubscriptionsMsg::SortByLatency(id) => {
                self.apply_edits(&sender, true, vec![SubscriptionEdit::SortByLatency(id)]);
            }
            SubscriptionsMsg::TestRealDelay(id) => {
                if self.tun_active {
                    let _ = sender.output(SubscriptionsOutput::Notice(
                        "Real Delay is unavailable while TUN is active".to_string(),
                    ));
                    return;
                }
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
                let targets = probe_targets(&node_refs);
                sender.oneshot_command(async move {
                    let per_batch = u64::from(real_delay_settings.timeout_ms) + 15_000;
                    let batches =
                        v2ray_rs_subscription::probe_batch_count(node_count).max(1) as u64;
                    let timeout =
                        std::time::Duration::from_millis(per_batch.saturating_mul(batches));
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
                    SubscriptionsCmdOutput::RealDelayResult(id, run_token, targets, report)
                });
            }
            SubscriptionsMsg::CheckAvailability(id) => {
                if self.tun_active {
                    let _ = sender.output(SubscriptionsOutput::Notice(
                        "Availability checks are unavailable while TUN is active".to_string(),
                    ));
                    return;
                }
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
                let targets = probe_targets(&node_refs);
                sender.oneshot_command(async move {
                    // Same per-node pass as Real Delay, plus one request per
                    // service per node through its tunnel.
                    let per_batch = u64::from(real_delay_settings.timeout_ms)
                        .saturating_add(15_000)
                        .saturating_add(30_000 * service_count as u64);
                    let batches =
                        v2ray_rs_subscription::probe_batch_count(node_count).max(1) as u64;
                    let timeout =
                        std::time::Duration::from_millis(per_batch.saturating_mul(batches));
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
                    SubscriptionsCmdOutput::AvailabilityResult(id, run_token, targets, report)
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
                self.apply_edits(&sender, true, vec![SubscriptionEdit::SortByRealDelay(id)]);
            }
            SubscriptionsMsg::EnableAllNodes(id) => {
                self.apply_edits(
                    &sender,
                    true,
                    vec![SubscriptionEdit::SetNodesEnabled {
                        subscription: id,
                        enabled: true,
                    }],
                );
            }
            SubscriptionsMsg::DisableAllNodes(id) => {
                self.apply_edits(
                    &sender,
                    true,
                    vec![SubscriptionEdit::SetNodesEnabled {
                        subscription: id,
                        enabled: false,
                    }],
                );
            }
            SubscriptionsMsg::DragDropSubscription(from, to) => {
                self.apply_edits(
                    &sender,
                    true,
                    vec![SubscriptionEdit::DragSubscription { from, to }],
                );
            }
            SubscriptionsMsg::DragDropNode(sub_id, from, to) => {
                self.apply_edits(
                    &sender,
                    true,
                    vec![SubscriptionEdit::DragNode {
                        subscription: sub_id,
                        from,
                        to,
                    }],
                );
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
            SubscriptionsMsg::ResetStorage => {
                if self.reset_pending {
                    return;
                }
                self.reset_pending = true;
                self.imports.invalidate();
                self.load_generation = self.load_generation.wrapping_add(1);
                let revision = self.writes.submit();
                let receipt = self.store.enqueue_reset_subscriptions();
                sender.oneshot_command(async move {
                    let result = match receipt.await {
                        Ok(result) => result.map(|_| ()),
                        Err(_) => Err(PersistenceError::CorruptConfig(writer_stopped_message())),
                    };
                    SubscriptionsCmdOutput::ResetSaved { revision, result }
                });
                return;
            }
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
                let real_delay_changed = self.real_delay_settings != real_delay_settings;
                let changed = self.auto_update_enabled != auto_update_enabled
                    || self.auto_update_interval_secs != auto_update_interval_secs;
                self.auto_update_enabled = auto_update_enabled;
                self.auto_update_interval_secs = auto_update_interval_secs;
                self.backend_type = backend_type;
                self.binary_path = binary_path;
                self.real_delay_settings = real_delay_settings;
                if backend_changed || binary_changed || real_delay_changed {
                    self.real_delay_run_token = self.real_delay_run_token.wrapping_add(1);
                    self.testing_real_delay.clear();
                    self.availability_run_token = self.availability_run_token.wrapping_add(1);
                    self.testing_availability.clear();
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
        match render_hint {
            RenderHint::NodeState => {
                refresh_node_widgets(
                    &self.node_widgets,
                    &self.subscriptions,
                    &self.egress_samples,
                );
                return;
            }
            RenderHint::NodeToggle(sub_id, idx) => {
                if update_node_switch(&self.node_widgets, sub_id, idx, &self.subscriptions) {
                    return;
                }
            }
            RenderHint::SubscriptionToggle(id) => {
                if update_subscription_toggle(&self.sub_expanders, id, &self.subscriptions) {
                    return;
                }
            }
            RenderHint::SubscriptionRename(id) => {
                if update_subscription_title(&self.sub_expanders, id, &self.subscriptions) {
                    return;
                }
            }
            RenderHint::Full => {}
        }

        render_list(
            &self.subscriptions,
            &self.list_container,
            &sender,
            &mut self.sub_expanders,
            &mut self.node_widgets,
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
        let mut render_hint = RenderHint::Full;
        match msg {
            SubscriptionsCmdOutput::AddDone {
                generation,
                outcome,
            } => {
                if !self.imports.accepts(generation) {
                    log::info!("discarding a subscription import that a reset invalidated");
                    return;
                }
                let summary =
                    format_update_summary(&outcome.subscription.name, "Imported", &outcome.result);
                let profile_available = outcome.subscription.imported_profile.is_some()
                    && matches!(outcome.subscription.source, SubscriptionSource::Url { .. });
                let name = outcome.subscription.name.clone();
                self.apply_edits(
                    &sender,
                    true,
                    vec![SubscriptionEdit::Push(outcome.subscription)],
                );
                let _ = sender.output(SubscriptionsOutput::Notice(summary));
                if profile_available {
                    let _ = sender.output(SubscriptionsOutput::Notice(format!(
                        "{name}: an imported routing & DNS profile is available; enable it in the subscription menu"
                    )));
                }
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
                    let changed_node_ids = changed_connection_node_ids(&previous, &merged);
                    let runtime_changed = !previous.runtime_state_eq(&merged);
                    self.apply_edits(
                        &sender,
                        runtime_changed,
                        vec![SubscriptionEdit::Replace(merged)],
                    );
                    if !changed_node_ids.is_empty() {
                        invalidate_node_samples(
                            &sender,
                            &self.store,
                            &mut self.egress_samples,
                            &mut self.verdicts,
                            id,
                            &changed_node_ids,
                        );
                    }
                    let _ = sender.output(SubscriptionsOutput::Notice(format_update_summary(
                        &name, "Updated", &result,
                    )));
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
            SubscriptionsCmdOutput::LatencyResult(id, targets, results) => {
                self.testing_latency.remove(&id);
                let mut latency_updates = Vec::new();
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    apply_latency_by_ids(sub, &targets, &results);
                    for (idx, target) in targets.iter().enumerate() {
                        let Some(value) = results.get(idx).copied().flatten() else {
                            continue;
                        };
                        if node_by_probe_target(sub, &targets, idx).is_none() {
                            continue;
                        }
                        latency_updates.push((target.node_id, value));
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
                render_hint = RenderHint::NodeState;
            }
            SubscriptionsCmdOutput::RealDelayResult(id, run_token, targets, report) => {
                if self.testing_real_delay.get(&id).copied() != Some(run_token) {
                    return;
                }
                self.testing_real_delay.remove(&id);
                let now = chrono::Utc::now();
                let mut delay_updates = Vec::new();
                let mut egress_updates = Vec::new();
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    apply_real_delay_by_ids(sub, &targets, &report.results);
                    for (idx, target) in targets.iter().enumerate() {
                        if node_by_probe_target(sub, &targets, idx).is_none() {
                            continue;
                        }
                        if let Some(value) = report.results.get(idx).copied().flatten() {
                            delay_updates.push((target.node_id, value));
                        }
                        if let Some(sample) = report.egress.get(idx).and_then(|e| e.as_ref()) {
                            self.egress_samples
                                .insert((id, target.node_id), sample.clone());
                            egress_updates.push((target.node_id, sample.clone()));
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
                render_hint = RenderHint::NodeState;
            }
            SubscriptionsCmdOutput::AvailabilityResult(id, run_token, targets, report) => {
                if self.testing_availability.get(&id).copied() != Some(run_token) {
                    return;
                }
                self.testing_availability.remove(&id);
                let now = chrono::Utc::now();
                let mut delay_updates = Vec::new();
                let mut egress_updates = Vec::new();
                let mut verdict_updates = Vec::new();
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    apply_real_delay_by_ids(sub, &targets, &report.results);
                    for (idx, target) in targets.iter().enumerate() {
                        if node_by_probe_target(sub, &targets, idx).is_none() {
                            continue;
                        }
                        if let Some(value) = report.results.get(idx).copied().flatten() {
                            delay_updates.push((target.node_id, value));
                        }
                        if let Some(sample) = report.egress.get(idx).and_then(|e| e.as_ref()) {
                            self.egress_samples
                                .insert((id, target.node_id), sample.clone());
                            egress_updates.push((target.node_id, sample.clone()));
                        }
                        if let Some(verdict_map) =
                            report.verdicts.get(idx).filter(|v| !v.is_empty())
                        {
                            self.verdicts
                                .insert((id, target.node_id), verdict_map.clone());
                            verdict_updates.push((target.node_id, verdict_map.clone()));
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
                render_hint = RenderHint::NodeState;
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
            SubscriptionsCmdOutput::AddFailed { generation, error } => {
                if !self.imports.accepts(generation) {
                    log::info!("discarding a subscription import failure that a reset invalidated");
                    return;
                }
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
                    let mut changed_samples: Vec<(Uuid, Vec<Uuid>)> = Vec::new();
                    let mut edits = Vec::new();
                    for (_, result) in &results {
                        if let Ok((updated, _)) = result
                            && let Some(existing) =
                                self.subscriptions.iter().find(|s| s.id == updated.id)
                        {
                            let merged = merge_refreshed_subscription(existing, updated.clone());
                            let changed_node_ids = changed_connection_node_ids(existing, &merged);
                            if !changed_node_ids.is_empty() {
                                changed_samples.push((existing.id, changed_node_ids));
                            }
                            edits.push(SubscriptionEdit::Replace(merged));
                        }
                    }
                    let mut changed = false;
                    for edit in &edits {
                        changed |= edit.apply(&mut self.subscriptions);
                    }
                    if changed {
                        let runtime_changed =
                            !subscriptions_runtime_state_eq(&previous, &self.subscriptions);
                        self.submit_edits(&sender, runtime_changed, edits);
                    }
                    invalidate_node_samples_batch(
                        &sender,
                        &self.store,
                        &mut self.egress_samples,
                        &mut self.verdicts,
                        &changed_samples,
                    );
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
            SubscriptionsCmdOutput::SubscriptionsSaved(outcome) => {
                let status = self
                    .writes
                    .complete(outcome.revision, outcome.result.is_ok());
                match outcome.result {
                    Ok(committed) => {
                        if status.durable && outcome.runtime {
                            let _ = sender.output(SubscriptionsOutput::SubscriptionsChanged);
                        }
                        let _ = sender.output(SubscriptionsOutput::SubscriptionsPersisted);
                        if status.latest {
                            self.report_active_nodes(
                                &sender,
                                committed.iter().any(|s| s.has_enabled_nodes()),
                            );
                        }
                    }
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsCmdOutput::ResetSaved { revision, result } => {
                let status = self.writes.complete(revision, result.is_ok());
                self.reset_pending = false;
                match result {
                    Ok(()) => {
                        self.subscriptions.clear();
                        self.egress_samples.clear();
                        self.verdicts.clear();
                        self.load_error = None;
                        let _ = sender.output(SubscriptionsOutput::SubscriptionsChanged);
                        let _ = sender.output(SubscriptionsOutput::SubscriptionsPersisted);
                        if status.latest {
                            self.report_active_nodes(&sender, false);
                        }
                    }
                    Err(err) => {
                        log::error!("reset subscriptions: {err}");
                        let _ = sender.output(SubscriptionsOutput::Notice(format!(
                            "Failed to reset subscriptions: {err}"
                        )));
                    }
                }
            }
            SubscriptionsCmdOutput::InitialLoaded(loaded) => {
                let InitialSubscriptions { generation, result } = loaded;
                if generation != self.load_generation {
                    return;
                }
                self.loading = false;
                match result {
                    Ok((subscriptions, latency)) => {
                        self.subscriptions = subscriptions;
                        if let Some(snapshot) = latency {
                            let (egress_samples, verdicts) =
                                hydrate_latency_state(&mut self.subscriptions, &snapshot);
                            self.egress_samples = egress_samples;
                            self.verdicts = verdicts;
                        }
                        self.load_error = None;
                        let has_active = self.subscriptions.iter().any(|s| s.has_enabled_nodes());
                        self.report_active_nodes(&sender, has_active);
                    }
                    Err(err) => {
                        log::error!("load subscriptions: {err}");
                        self.load_error = Some(format!("Subscriptions are read-only: {err}"));
                    }
                }
            }
            SubscriptionsCmdOutput::SnapshotSaved(result) => {
                if let Err(err) = result {
                    log::error!("save latency snapshot: {err}");
                    let _ = sender.output(SubscriptionsOutput::Notice(format!(
                        "Failed to save latency snapshot: {err}"
                    )));
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
        if matches!(render_hint, RenderHint::NodeState) {
            refresh_node_widgets(
                &self.node_widgets,
                &self.subscriptions,
                &self.egress_samples,
            );
            return;
        }
        render_list(
            &self.subscriptions,
            &self.list_container,
            &sender,
            &mut self.sub_expanders,
            &mut self.node_widgets,
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

impl SubscriptionsPage {
    /// Applies edits to the local list in submission order, then enqueues the
    /// same ordered transaction on the shared writer.
    fn apply_edits(
        &mut self,
        sender: &ComponentSender<Self>,
        runtime: bool,
        edits: Vec<SubscriptionEdit>,
    ) -> bool {
        let mut changed = false;
        for edit in &edits {
            changed |= edit.apply(&mut self.subscriptions);
        }
        if changed {
            self.submit_edits(sender, runtime, edits);
        }
        changed
    }

    fn submit_edits(
        &mut self,
        sender: &ComponentSender<Self>,
        runtime: bool,
        edits: Vec<SubscriptionEdit>,
    ) {
        let revision = self.writes.submit();
        let receipt = self
            .store
            .enqueue_update_subscriptions(move |subscriptions| {
                for edit in edits {
                    edit.apply(subscriptions);
                }
            });
        sender.oneshot_command(async move {
            let result = match receipt.await {
                Ok(result) => result.map(|(_, committed)| committed),
                Err(_) => Err(PersistenceError::CorruptConfig(writer_stopped_message())),
            };
            SubscriptionsCmdOutput::SubscriptionsSaved(SubscriptionSaveOutcome {
                revision,
                runtime,
                result,
            })
        });
    }

    fn report_active_nodes(&mut self, sender: &ComponentSender<Self>, has_active: bool) {
        if has_active == self.active_nodes_reported {
            return;
        }
        self.active_nodes_reported = has_active;
        let _ = sender.output(SubscriptionsOutput::ActiveNodesChanged(has_active));
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

/// Node ids whose connection settings changed between the previous state and
/// the merged refresh; remark-only edits do not count. Reconciled nodes keep
/// their id, so the ids drive latency snapshot invalidation.
fn changed_connection_node_ids(current: &Subscription, merged: &Subscription) -> Vec<Uuid> {
    merged
        .nodes
        .iter()
        .filter_map(|node| {
            let previous = current.nodes.iter().find(|old| old.id == node.id)?;
            (!same_connection(&previous.node, &node.node)).then_some(node.id)
        })
        .collect()
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

const V2RAYTUN_IMPORT_PREFIX: &str = "v2raytun://import/";

#[derive(Debug, Clone)]
pub(crate) enum SubscriptionSourceInput {
    Url(String),
    File(String),
    Paste(String),
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
mod persistence_order_tests {
    use super::*;
    use std::sync::Arc;
    use v2ray_rs_core::persistence::AppPaths;
    use v2ray_rs_core::profile::AppProfile;

    fn workspace() -> (tempfile::TempDir, WorkspaceStore) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        paths.ensure_dirs().unwrap();
        (tmp, WorkspaceStore::new(paths))
    }

    fn seed(store: &WorkspaceStore) {
        store
            .save_subscriptions(&[Subscription::new_from_url(
                "Seed",
                "https://example.com/seed",
            )])
            .unwrap();
    }

    #[test]
    fn a_reset_invalidates_an_in_flight_import() {
        let mut imports = ImportGeneration::default();
        let captured = imports.current();
        assert!(imports.accepts(captured));

        imports.invalidate();

        assert!(!imports.accepts(captured));
        assert!(imports.accepts(imports.current()));
    }

    #[test]
    fn receipts_before_the_latest_submission_are_not_latest() {
        let mut writes = WriteTracker::default();
        let first = writes.submit();
        let second = writes.submit();

        let first_status = writes.complete(first, true);
        assert!(first_status.durable);
        assert!(!first_status.latest);

        let second_status = writes.complete(second, false);
        assert!(!second_status.durable);
        assert!(second_status.latest);
        assert_eq!(writes.committed, first);
    }

    #[tokio::test]
    async fn held_edit_keeps_the_committed_snapshot_and_the_later_edit_wins() {
        let (_tmp, store) = workspace();
        seed(&store);
        let before = store.subscriptions_snapshot().unwrap();
        let disk_before = std::fs::read_to_string(store.paths().subscriptions_path()).unwrap();
        let latch = store.test_writer_latch().clone();

        latch.arm();
        let first = store.enqueue_update_subscriptions(|subscriptions| {
            subscriptions[0].name = "First".to_string();
        });
        latch.wait_until_parked();
        let second = store.enqueue_update_subscriptions(|subscriptions| {
            subscriptions[0].name = "Second".to_string();
        });

        assert!(Arc::ptr_eq(
            &before,
            &store.subscriptions_snapshot().unwrap()
        ));
        assert_eq!(
            std::fs::read_to_string(store.paths().subscriptions_path()).unwrap(),
            disk_before,
            "a held edit must not reach disk before its receipt"
        );

        latch.release();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        assert_eq!(store.subscriptions_snapshot().unwrap()[0].name, "Second");
        assert_eq!(
            WorkspaceStore::new(store.paths().clone())
                .load_subscriptions()
                .unwrap()[0]
                .name,
            "Second",
            "the later edit must survive a reopen"
        );
    }

    #[tokio::test]
    async fn a_failed_edit_keeps_the_previous_committed_snapshot() {
        let (_tmp, store) = workspace();
        seed(&store);
        let latch = store.test_writer_latch().clone();
        let data_dir = store
            .paths()
            .subscriptions_path()
            .parent()
            .unwrap()
            .to_path_buf();

        latch.arm();
        let failing = store.enqueue_update_subscriptions(|subscriptions| {
            subscriptions[0].name = "Failed".to_string();
        });
        latch.wait_until_parked();
        std::fs::remove_dir_all(&data_dir).unwrap();
        std::fs::write(&data_dir, b"not a directory").unwrap();
        latch.release();
        assert!(
            failing.await.unwrap().is_err(),
            "a storage failure must answer with an explicit error"
        );
        assert_eq!(store.subscriptions_snapshot().unwrap()[0].name, "Seed");

        std::fs::remove_file(&data_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        latch.arm();
        let later = store.enqueue_update_subscriptions(|subscriptions| {
            subscriptions[0].name = "Later".to_string();
        });
        latch.wait_until_parked();
        latch.release();
        later.await.unwrap().unwrap();

        assert_eq!(store.subscriptions_snapshot().unwrap()[0].name, "Later");
        assert_eq!(
            WorkspaceStore::new(store.paths().clone())
                .load_subscriptions()
                .unwrap()[0]
                .name,
            "Later",
            "the failed edit must not survive a reopen"
        );
    }

    #[tokio::test]
    async fn a_queued_save_before_reset_leaves_no_file_and_reopen_is_empty() {
        let (_tmp, store) = workspace();
        let latch = store.test_writer_latch().clone();

        latch.arm();
        let save = store.enqueue_save_subscriptions(vec![Subscription::new_from_url(
            "One",
            "https://example.com/one",
        )]);
        latch.wait_until_parked();
        let reset = store.enqueue_reset_subscriptions();
        latch.release();

        save.await.unwrap().unwrap();
        reset.await.unwrap().unwrap();

        assert!(store.subscriptions_snapshot().unwrap().is_empty());
        assert!(!store.paths().subscriptions_path().exists());
        assert!(
            WorkspaceStore::new(store.paths().clone())
                .load_subscriptions()
                .unwrap()
                .is_empty(),
            "reset must not be recreated by the earlier save on reopen"
        );
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
                {
                    let mut node = current.nodes[0].clone();
                    node.enabled = true;
                    node
                },
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
        let targets = probe_targets(&sub.nodes);
        let results = vec![Some(30), None, Some(70)];

        sub.nodes.swap(0, 2);
        apply_latency_by_ids(&mut sub, &targets, &results);

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
        let targets = probe_targets(&sub.nodes);
        let results = vec![Some(11), Some(22)];

        sub.nodes.remove(1);
        apply_latency_by_ids(&mut sub, &targets, &results);

        assert_eq!(sub.nodes[0].last_latency_ms, Some(11));
        assert!(
            sub.nodes
                .iter()
                .all(|n| n.id == targets[0].node_id || n.last_latency_ms.is_none())
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
        let targets = probe_targets(&sub.nodes);
        let results = vec![Some(120), None];

        sub.nodes.reverse();
        apply_real_delay_by_ids(&mut sub, &targets, &results);

        assert_eq!(sub.nodes[1].last_real_delay_ms, Some(120));
        assert_eq!(sub.nodes[0].last_real_delay_ms, None);
    }

    #[test]
    fn probe_result_for_changed_connection_is_dropped() {
        let mut sub = create_test_subscription(
            "Rotated",
            true,
            vec![create_test_node("10.0.0.1", 1001, true)],
        );
        let node_id = sub.nodes[0].id;
        let captured = sub.nodes[0].node.clone();

        // A refresh swapped the credentials behind the same node id.
        if let ProxyNode::Vless(config) = &mut sub.nodes[0].node {
            config.uuid = "rotated-credentials".into();
        }

        let targets = vec![ProbeTarget {
            node_id,
            connection: captured,
        }];
        apply_latency_by_ids(&mut sub, &targets, &[Some(42)]);
        apply_real_delay_by_ids(&mut sub, &targets, &[Some(120)]);

        assert_eq!(
            sub.nodes[0].last_latency_ms, None,
            "a stale probe result must not attach to a changed connection"
        );
        assert_eq!(sub.nodes[0].last_real_delay_ms, None);

        // A remark-only rename is not a connection change, so results apply.
        let rotated = sub.nodes[0].node.clone();
        let mut renamed = sub.nodes[0].clone();
        if let ProxyNode::Vless(config) = &mut renamed.node {
            config.remark = Some("renamed".into());
        }
        sub.nodes[0] = renamed;
        let remark_targets = vec![ProbeTarget {
            node_id,
            connection: rotated,
        }];
        apply_latency_by_ids(&mut sub, &remark_targets, &[Some(42)]);
        assert_eq!(sub.nodes[0].last_latency_ms, Some(42));
    }

    #[test]
    fn refresh_merge_reports_changed_connection_for_snapshot_invalidation() {
        let current = create_test_subscription(
            "Corp",
            true,
            vec![
                create_test_node("10.0.0.1", 1001, true),
                create_test_node("10.0.0.2", 1002, true),
            ],
        );
        let changed_id = current.nodes[0].id;
        let stable_id = current.nodes[1].id;

        let mut changed_node = current.nodes[0].clone();
        if let ProxyNode::Vless(config) = &mut changed_node.node {
            config.uuid = "rotated-credentials".into();
        }
        let refreshed = Subscription {
            id: current.id,
            name: current.name.clone(),
            source: current.source.clone(),
            nodes: vec![changed_node, current.nodes[1].clone()],
            last_updated: Some(chrono::Utc::now()),
            auto_update_interval_secs: None,
            enabled: true,
            imported_profile: None,
            use_imported_profile: true,
        };

        let merged = merge_refreshed_subscription(&current, refreshed);

        assert_eq!(
            merged.nodes[0].id, changed_id,
            "reconcile must keep the node id"
        );
        assert_eq!(
            changed_connection_node_ids(&current, &merged),
            vec![changed_id],
            "the changed connection must be reported for snapshot invalidation"
        );
        assert_eq!(merged.nodes[1].id, stable_id);
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

    #[test]
    fn subscription_form_requires_a_name_and_exactly_one_source() {
        assert_eq!(
            subscription_form_error("", "https://example.com/sub", "", ""),
            Some("A subscription name is required".to_string())
        );
        assert_eq!(
            subscription_form_error("Work", "", "", ""),
            Some("Provide a URL, a local file path, or pasted JSON".to_string())
        );
        assert_eq!(
            subscription_form_error("Work", "https://example.com/sub", "/tmp/sub.json", ""),
            Some("Provide exactly one subscription source".to_string())
        );
        assert_eq!(
            subscription_form_error("Work", "https://example.com/sub", "", ""),
            None
        );
        assert_eq!(
            subscription_form_error("Work", "", "/tmp/sub.json", ""),
            None
        );
        assert_eq!(subscription_form_error("Work", "", "", "{}"), None);
    }

    #[test]
    fn v2raytun_prefix_is_stripped_before_source_detection() {
        use base64::Engine as _;

        let encoded = base64::engine::general_purpose::STANDARD.encode("https://example.com/sub");
        let input = format!("{V2RAYTUN_IMPORT_PREFIX}{encoded}");
        assert_eq!(
            strip_v2raytun_import_prefix(&input),
            "https://example.com/sub"
        );
        assert_eq!(subscription_form_error("Work", &input, "", ""), None);
    }
}
