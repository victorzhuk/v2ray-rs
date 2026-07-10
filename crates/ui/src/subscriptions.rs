use adw::prelude::*;
use gtk::gdk;
use relm4::adw;
use relm4::prelude::*;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use v2ray_rs_core::models::{
    AppSettings, BackendType, RealDelayCapability, RealDelaySettings, Subscription,
    SubscriptionSource,
};
use v2ray_rs_core::persistence::{AppPaths, PersistenceError};
use v2ray_rs_core::runtime_snapshot::subscriptions_runtime_state_eq;
use v2ray_rs_subscription::{
    RealDelayReport, SubscriptionError, SubscriptionImportOutcome, SubscriptionService,
    UpdateError, UpdateResult, reconcile_nodes,
};

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
    testing_latency: HashSet<Uuid>,
    testing_real_delay: HashMap<Uuid, u64>,
    real_delay_run_token: u64,
    expanded_subs: HashSet<Uuid>,
    active_node: Option<(Uuid, Uuid)>,
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
    ToggleNode(Uuid, usize),
    DeleteSubscription(Uuid),
    RenameSubscription(Uuid, String),
    MoveSubscription(Uuid, Direction),
    MoveNode(Uuid, usize, Direction),
    AddSubscription(String, SubscriptionSource),
    UpdateSubscription(Uuid),
    TestLatency(Uuid),
    SortByLatency(Uuid),
    TestRealDelay(Uuid),
    SortByRealDelay(Uuid),
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
    ExpanderToggled(Uuid, bool),
}

#[derive(Debug)]
pub enum SubscriptionsCmdOutput {
    AddDone(SubscriptionImportOutcome),
    RefreshDone(Subscription, UpdateResult),
    LatencyResult(Uuid, Vec<Option<u64>>),
    RealDelayResult(Uuid, u64, RealDelayReport),
    RefreshFailed(Uuid, SubscriptionError),
    AddFailed(SubscriptionError),
    AutoUpdateDone(AutoUpdateRefreshBatch),
    AutoUpdateTick(u64),
    BackgroundLatencyTick,
}

fn commit_subscriptions_mutation<R>(
    store: &WorkspaceStore,
    subscriptions: &mut Vec<Subscription>,
    mutate: impl FnOnce(&mut Vec<Subscription>) -> Option<R>,
) -> Result<Option<R>, PersistenceError> {
    let mut next = subscriptions.clone();
    let result = mutate(&mut next);
    let Some(result) = result else {
        return Ok(None);
    };

    store.save_subscriptions(&next)?;
    *subscriptions = next;
    Ok(Some(result))
}

fn report_subscription_persist_error(
    sender: &ComponentSender<SubscriptionsPage>,
    err: &PersistenceError,
) {
    log::error!("save subscriptions: {err}");
    let _ = sender.output(SubscriptionsOutput::Notice(format!(
        "Failed to save subscriptions: {err}"
    )));
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
            testing_latency: HashSet::new(),
            testing_real_delay: HashMap::new(),
            real_delay_run_token: 0,
            expanded_subs: HashSet::new(),
            active_node: None,
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
                    | SubscriptionsMsg::SyncSettings { .. }
            )
        {
            return;
        }

        let mut subscriptions_changed = false;
        let mut render_hint = RenderHint::Full;

        match msg {
            SubscriptionsMsg::ToggleSubscription(id) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                        sub.enabled = !sub.enabled;
                        Some(RenderHint::SubscriptionToggle(id))
                    },
                ) {
                    Ok(Some(hint)) => {
                        subscriptions_changed = true;
                        render_hint = hint;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::ToggleNode(sub_id, idx) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == sub_id)?;
                        let node = sub.nodes.get_mut(idx)?;
                        node.enabled = !node.enabled;
                        Some(RenderHint::NodeToggle(sub_id, idx))
                    },
                ) {
                    Ok(Some(hint)) => {
                        subscriptions_changed = true;
                        render_hint = hint;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::RenameSubscription(id, new_name) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                        sub.name = new_name;
                        Some(RenderHint::SubscriptionRename(id))
                    },
                ) {
                    Ok(Some(hint)) => {
                        render_hint = hint;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::MoveSubscription(id, direction) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
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
                    },
                ) {
                    Ok(Some(())) => {
                        subscriptions_changed = true;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::MoveNode(sub_id, idx, direction) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
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
                    },
                ) {
                    Ok(Some(())) => {
                        subscriptions_changed = true;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::DeleteSubscription(id) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
                        let initial_len = subscriptions.len();
                        subscriptions.retain(|s| s.id != id);
                        (subscriptions.len() != initial_len).then_some(())
                    },
                ) {
                    Ok(Some(())) => {
                        self.expanded_subs.remove(&id);
                        subscriptions_changed = true;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::AddSubscription(name, source) => {
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
                let svc = self.service.clone();
                let subscription = match self.subscriptions.iter().find(|s| s.id == id) {
                    Some(subscription) => subscription.clone(),
                    None => return,
                };
                sender.oneshot_command(async move {
                    match svc.refresh(subscription).await {
                        Ok((sub, result)) => SubscriptionsCmdOutput::RefreshDone(sub, result),
                        Err(e) => SubscriptionsCmdOutput::RefreshFailed(id, e),
                    }
                });
                return;
            }
            SubscriptionsMsg::TestLatency(id) => {
                if self.testing_latency.contains(&id) {
                    return;
                }
                let sub = match self.subscriptions.iter().find(|s| s.id == id) {
                    Some(s) => s.clone(),
                    None => return,
                };
                self.testing_latency.insert(id);
                let nodes = sub.nodes.clone();
                sender.oneshot_command(async move {
                    let results = v2ray_rs_subscription::ping_nodes(&nodes).await;
                    SubscriptionsCmdOutput::LatencyResult(id, results)
                });
                return;
            }
            SubscriptionsMsg::SortByLatency(id) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                        sub.nodes.sort_by(|a, b| {
                            let la = a.last_latency_ms.unwrap_or(u64::MAX);
                            let lb = b.last_latency_ms.unwrap_or(u64::MAX);
                            la.cmp(&lb)
                        });
                        Some(())
                    },
                ) {
                    Ok(Some(())) => {
                        subscriptions_changed = true;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
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
                            diagnostic: Some("Real Delay probe timed out".to_string()),
                        },
                    };
                    SubscriptionsCmdOutput::RealDelayResult(id, run_token, report)
                });
                return;
            }
            SubscriptionsMsg::SortByRealDelay(id) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                        sub.nodes.sort_by(|a, b| {
                            let la = a.last_real_delay_ms.unwrap_or(u64::MAX);
                            let lb = b.last_real_delay_ms.unwrap_or(u64::MAX);
                            la.cmp(&lb)
                        });
                        Some(())
                    },
                ) {
                    Ok(Some(())) => {
                        subscriptions_changed = true;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::EnableAllNodes(id) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                        for node in &mut sub.nodes {
                            node.enabled = true;
                        }
                        Some(())
                    },
                ) {
                    Ok(Some(())) => {
                        subscriptions_changed = true;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::DisableAllNodes(id) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == id)?;
                        for node in &mut sub.nodes {
                            node.enabled = false;
                        }
                        Some(())
                    },
                ) {
                    Ok(Some(())) => {
                        subscriptions_changed = true;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::DragDropSubscription(from, to) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
                        if from == to || from >= subscriptions.len() || to >= subscriptions.len() {
                            return None;
                        }
                        let sub = subscriptions.remove(from);
                        subscriptions.insert(to, sub);
                        Some(())
                    },
                ) {
                    Ok(Some(())) => {
                        subscriptions_changed = true;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::DragDropNode(sub_id, from, to) => {
                match commit_subscriptions_mutation(
                    &self.store,
                    &mut self.subscriptions,
                    |subscriptions| {
                        let sub = subscriptions.iter_mut().find(|s| s.id == sub_id)?;
                        if from == to || from >= sub.nodes.len() || to >= sub.nodes.len() {
                            return None;
                        }
                        let node = sub.nodes.remove(from);
                        sub.nodes.insert(to, node);
                        Some(())
                    },
                ) {
                    Ok(Some(())) => {
                        subscriptions_changed = true;
                    }
                    Ok(None) => {}
                    Err(err) => report_subscription_persist_error(&sender, &err),
                }
            }
            SubscriptionsMsg::SetActiveNode(active) => {
                if self.active_node == active {
                    return;
                }
                self.active_node = active;
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
                self.subscriptions.push(outcome.subscription.clone());
                if let Err(err) = self.store.save_subscriptions(&self.subscriptions) {
                    log::error!("save subscriptions: {err}");
                    let _ = self.subscriptions.pop();
                    let _ = sender.output(SubscriptionsOutput::Notice(format!(
                        "Failed to save imported subscription: {err}"
                    )));
                } else {
                    let _ = sender.output(SubscriptionsOutput::SubscriptionsChanged);
                    let _ = sender.output(SubscriptionsOutput::Notice(summary));
                }
            }
            SubscriptionsCmdOutput::RefreshDone(sub, result) => {
                let id = sub.id;
                if let Some(pos) = self.subscriptions.iter().position(|s| s.id == id) {
                    let name = self.subscriptions[pos].name.clone();
                    let previous = self.subscriptions[pos].clone();
                    let merged = merge_refreshed_subscription(&previous, sub);
                    let runtime_changed = !previous.runtime_state_eq(&merged);
                    self.subscriptions[pos] = merged;
                    if let Err(err) = self.store.save_subscriptions(&self.subscriptions) {
                        log::error!("save subscriptions: {err}");
                        self.subscriptions[pos] = previous;
                        let _ = sender.output(SubscriptionsOutput::Notice(format!(
                            "Failed to save updated subscription: {err}"
                        )));
                    } else {
                        if runtime_changed {
                            let _ = sender.output(SubscriptionsOutput::SubscriptionsChanged);
                        }
                        if !result.parse_failures.is_empty() {
                            let _ = sender.output(SubscriptionsOutput::Notice(
                                format_update_summary(&name, "Updated", &result),
                            ));
                        }
                    }
                }
                log::info!(
                    "updated subscription {id}: +{} -{} ={} failed={}",
                    result.added,
                    result.removed,
                    result.unchanged,
                    result.parse_failures.len()
                );
            }
            SubscriptionsCmdOutput::LatencyResult(id, results) => {
                self.testing_latency.remove(&id);
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    for (node, latency) in sub.nodes.iter_mut().zip(results.iter()) {
                        node.last_latency_ms = *latency;
                    }
                    if let Ok(mut snapshot) = self.store.load_latency_snapshot() {
                        let now = chrono::Utc::now();
                        for (node, latency) in sub.nodes.iter().zip(results.iter()) {
                            if let Some(value) = latency {
                                let node_ref =
                                    v2ray_rs_core::models::ConnectionNodeRef::Subscription {
                                        subscription_id: sub.id,
                                        node_id: node.id,
                                    };
                                snapshot.upsert(node_ref, *value, now);
                            }
                        }
                        if let Err(e) = self.store.save_latency_snapshot(&snapshot) {
                            log::error!("save latency snapshot: {e}");
                        }
                    }
                }
            }
            SubscriptionsCmdOutput::RealDelayResult(id, run_token, report) => {
                if self.testing_real_delay.get(&id).copied() != Some(run_token) {
                    return;
                }
                self.testing_real_delay.remove(&id);
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    for (node, delay) in sub.nodes.iter_mut().zip(report.results.iter()) {
                        node.last_real_delay_ms = *delay;
                    }
                    if let Ok(mut snapshot) = self.store.load_latency_snapshot() {
                        let now = chrono::Utc::now();
                        for (node, delay) in sub.nodes.iter().zip(report.results.iter()) {
                            if let Some(value) = delay {
                                let node_ref =
                                    v2ray_rs_core::models::ConnectionNodeRef::Subscription {
                                        subscription_id: sub.id,
                                        node_id: node.id,
                                    };
                                snapshot.upsert_real_delay(node_ref, *value, now);
                            }
                        }
                        if let Err(e) = self.store.save_latency_snapshot(&snapshot) {
                            log::error!("save latency snapshot: {e}");
                        }
                    }
                }

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
            SubscriptionsCmdOutput::RefreshFailed(id, error) => {
                log::error!("failed to update subscription {id}: {error}");
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
                    if let Err(err) = self.store.save_subscriptions(&self.subscriptions) {
                        log::error!("save subscriptions: {err}");
                        self.subscriptions = previous;
                        let _ = sender.output(SubscriptionsOutput::Notice(format!(
                            "Failed to save auto-updated subscriptions: {err}"
                        )));
                    } else {
                        let runtime_changed =
                            !subscriptions_runtime_state_eq(&previous, &self.subscriptions);
                        if runtime_changed {
                            let _ = sender.output(SubscriptionsOutput::SubscriptionsChanged);
                        }
                        for (id, result) in &results {
                            match result {
                                Ok((_, r)) => log::info!(
                                    "auto-updated {id}: +{} -{} ={}",
                                    r.added,
                                    r.removed,
                                    r.unchanged
                                ),
                                Err(e) => log::warn!("auto-update {id} failed: {e}"),
                            }
                        }
                    }
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
                );
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
            },
        );
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
    }
}

fn format_update_summary(name: &str, action: &str, result: &UpdateResult) -> String {
    if result.parse_failures.is_empty() {
        format!(
            "{action} {name}: +{} -{} unchanged {}",
            result.added, result.removed, result.unchanged
        )
    } else {
        let failed = result
            .parse_failures
            .iter()
            .take(2)
            .map(|failure| failure.uri.as_str())
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
    }
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
                    .map(|failure| failure.uri.as_str())
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
) -> Vec<Uuid> {
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
    let menu = build_subscription_menu(
        sub,
        sender,
        state.testing_latency.contains(&sub.id),
        state.testing_real_delay.contains_key(&sub.id),
        state.real_delay_available,
        state.real_delay_capability,
    );
    expander.add_suffix(&menu);

    for (idx, node) in sub.nodes.iter().enumerate() {
        let active = state.active_node == Some((sub.id, node.id));
        expander.add_row(&build_node_row(
            sub.id,
            idx,
            node,
            sender,
            state.locked,
            active,
        ));
    }

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

fn build_subscription_menu(
    sub: &Subscription,
    sender: &ComponentSender<SubscriptionsPage>,
    is_testing: bool,
    is_testing_real_delay: bool,
    real_delay_available: bool,
    real_delay_capability: &RealDelayCapability,
) -> gtk::MenuButton {
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
        .sensitive(!is_testing)
        .build();
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
        .subtitle(&address)
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

    dialog.set_extra_child(Some(&content));

    dialog.connect_response(None, move |_, response| {
        if response == "add" {
            let name = name_entry.text().to_string();
            let url = url_entry.text().to_string();
            let file_path = file_entry.text().to_string();
            if !name.trim().is_empty()
                && let Some(source) = subscription_source_from_inputs(url.trim(), file_path.trim())
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

pub(crate) fn subscription_source_from_inputs(
    url: &str,
    file_path: &str,
) -> Option<SubscriptionSource> {
    match (url.trim().is_empty(), file_path.trim().is_empty()) {
        (false, true) => Some(SubscriptionSource::Url {
            url: url.trim().into(),
        }),
        (true, false) => Some(SubscriptionSource::File {
            path: file_path.trim().into(),
        }),
        _ => None,
    }
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
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use v2ray_rs_core::models::{ProxyNode, SubscriptionNode, VlessConfig};

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
    fn enabled_subscription_with_enabled_nodes_eligible() {
        let sub = create_test_subscription(
            "Test Sub",
            true,
            vec![create_test_node("127.0.0.1", 8080, true)],
        );
        let testing_latency = HashSet::new();

        let eligible = subscriptions_eligible_for_latency_test(&[sub], &testing_latency);

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

        let eligible = subscriptions_eligible_for_latency_test(&[sub], &testing_latency);

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

        let eligible = subscriptions_eligible_for_latency_test(&[sub], &testing_latency);

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

        let eligible = subscriptions_eligible_for_latency_test(&[sub], &testing_latency);

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

        let eligible =
            subscriptions_eligible_for_latency_test(&[sub1, sub2, sub3, sub4], &testing_latency);

        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0], sub1_id);
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
}
