use adw::prelude::*;
use gtk::gdk;
use relm4::adw;
use relm4::prelude::*;
use std::collections::HashSet;
use uuid::Uuid;

use v2ray_rs_core::models::{AppSettings, Subscription, SubscriptionSource};
use v2ray_rs_core::persistence::{self, AppPaths};
use v2ray_rs_subscription::manager::SubscriptionService;
use v2ray_rs_subscription::update::UpdateResult;

pub struct SubscriptionsPage {
    paths: AppPaths,
    service: SubscriptionService,
    subscriptions: Vec<Subscription>,
    list_container: gtk::ListBox,
    auto_update_interval_secs: u64,
    testing_latency: HashSet<Uuid>,
    locked: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
}

#[derive(Debug)]
pub enum SubscriptionsOutput {
    ActiveNodesChanged(bool),
}

#[derive(Debug)]
pub enum SubscriptionsMsg {
    Refresh,
    ToggleSubscription(Uuid),
    ToggleNode(Uuid, usize),
    DeleteSubscription(Uuid),
    RenameSubscription(Uuid, String),
    MoveSubscription(Uuid, Direction),
    MoveNode(Uuid, usize, Direction),
    AddSubscription(String, String),
    UpdateSubscription(Uuid),
    TestLatency(Uuid),
    SortByLatency(Uuid),
    EnableAllNodes(Uuid),
    DisableAllNodes(Uuid),
    DragDropSubscription(usize, usize),
    DragDropNode(Uuid, usize, usize),
    CheckAutoUpdate,
    SetLocked(bool),
}

#[derive(Debug)]
pub enum SubscriptionsCmdOutput {
    RefreshDone(Uuid, Subscription, UpdateResult),
    LatencyResult(Uuid, Vec<Option<u64>>),
    RefreshFailed(Uuid, String),
    AutoUpdateDone(Vec<(Uuid, Result<UpdateResult, String>)>),
}

#[relm4::component(pub)]
impl Component for SubscriptionsPage {
    type Init = (AppPaths, AppSettings);
    type Input = SubscriptionsMsg;
    type Output = SubscriptionsOutput;
    type CommandOutput = SubscriptionsCmdOutput;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 0,

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
                    set_sensitive: !model.locked,
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
        let (paths, settings) = init;
        let service = SubscriptionService::new(paths.clone());
        let subscriptions = persistence::load_subscriptions(&paths).unwrap_or_default();

        let list_container = gtk::ListBox::builder()
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["boxed-list"])
            .selection_mode(gtk::SelectionMode::None)
            .build();

        let model = SubscriptionsPage {
            paths,
            service,
            subscriptions,
            list_container: list_container.clone(),
            auto_update_interval_secs: settings.subscription_update_interval_secs,
            testing_latency: HashSet::new(),
            locked: false,
        };

        render_list(&model.subscriptions, &list_container, &sender, &HashSet::new(), &HashSet::new(), false);

        if settings.auto_update_subscriptions {
            sender.input(SubscriptionsMsg::CheckAutoUpdate);
        }

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        let emit_active_nodes = |subs: &[Subscription], sender: &ComponentSender<Self>| {
            let has_active = subs.iter().any(|s| s.has_enabled_nodes());
            let _ = sender.output(SubscriptionsOutput::ActiveNodesChanged(has_active));
        };

        match msg {
            SubscriptionsMsg::Refresh => {
                self.subscriptions =
                    persistence::load_subscriptions(&self.paths).unwrap_or_default();
            }
            SubscriptionsMsg::ToggleSubscription(id) => {
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    sub.enabled = !sub.enabled;
                    if let Err(e) = persistence::update_subscription(&self.paths, sub.clone()) {
                        log::error!("update subscription: {e}");
                    }
                }
            }
            SubscriptionsMsg::ToggleNode(sub_id, idx) => {
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == sub_id) {
                    if let Some(node) = sub.nodes.get_mut(idx) {
                        node.enabled = !node.enabled;
                        if let Err(e) = persistence::update_subscription(&self.paths, sub.clone()) {
                            log::error!("update subscription: {e}");
                        }
                    }
                }
            }
            SubscriptionsMsg::RenameSubscription(id, new_name) => {
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    sub.name = new_name;
                    if let Err(e) = persistence::update_subscription(&self.paths, sub.clone()) {
                        log::error!("update subscription: {e}");
                    }
                }
            }
            SubscriptionsMsg::MoveSubscription(id, direction) => {
                if let Some(pos) = self.subscriptions.iter().position(|s| s.id == id) {
                    let new_pos = match direction {
                        Direction::Up if pos > 0 => pos - 1,
                        Direction::Down if pos + 1 < self.subscriptions.len() => pos + 1,
                        _ => pos,
                    };
                    if new_pos != pos {
                        self.subscriptions.swap(pos, new_pos);
                        if let Err(e) = persistence::save_subscriptions(&self.paths, &self.subscriptions) {
                            log::error!("save subscriptions: {e}");
                        }
                    }
                }
            }
            SubscriptionsMsg::MoveNode(sub_id, idx, direction) => {
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == sub_id) {
                    let new_idx = match direction {
                        Direction::Up if idx > 0 => idx - 1,
                        Direction::Down if idx + 1 < sub.nodes.len() => idx + 1,
                        _ => idx,
                    };
                    if new_idx != idx {
                        sub.nodes.swap(idx, new_idx);
                        if let Err(e) = persistence::update_subscription(&self.paths, sub.clone()) {
                            log::error!("update subscription: {e}");
                        }
                    }
                }
            }
            SubscriptionsMsg::DeleteSubscription(id) => {
                if let Err(e) = persistence::remove_subscription(&self.paths, &id) {
                    log::error!("remove subscription: {e}");
                }
                self.subscriptions.retain(|s| s.id != id);
            }
            SubscriptionsMsg::AddSubscription(name, url) => {
                let sub = Subscription::new_from_url(name, url);
                let id = sub.id;
                if let Err(e) = persistence::add_subscription(&self.paths, sub.clone()) {
                    log::error!("add subscription: {e}");
                }
                self.subscriptions.push(sub);
                sender.input(SubscriptionsMsg::UpdateSubscription(id));
            }
            SubscriptionsMsg::UpdateSubscription(id) => {
                let svc = self.service.clone();
                sender.oneshot_command(async move {
                    match svc.refresh(id).await {
                        Ok((sub, result)) => {
                            SubscriptionsCmdOutput::RefreshDone(id, sub, result)
                        }
                        Err(e) => SubscriptionsCmdOutput::RefreshFailed(id, e.to_string()),
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
                    let results = v2ray_rs_subscription::ping::ping_nodes(&nodes).await;
                    SubscriptionsCmdOutput::LatencyResult(id, results)
                });
                return;
            }
            SubscriptionsMsg::SortByLatency(id) => {
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    sub.nodes.sort_by(|a, b| {
                        let la = a.last_latency_ms.unwrap_or(u64::MAX);
                        let lb = b.last_latency_ms.unwrap_or(u64::MAX);
                        la.cmp(&lb)
                    });
                    if let Err(e) = persistence::update_subscription(&self.paths, sub.clone()) {
                        log::error!("update subscription: {e}");
                    }
                }
            }
            SubscriptionsMsg::EnableAllNodes(id) => {
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    for node in &mut sub.nodes {
                        node.enabled = true;
                    }
                    if let Err(e) = persistence::update_subscription(&self.paths, sub.clone()) {
                        log::error!("update subscription: {e}");
                    }
                }
            }
            SubscriptionsMsg::DisableAllNodes(id) => {
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    for node in &mut sub.nodes {
                        node.enabled = false;
                    }
                    if let Err(e) = persistence::update_subscription(&self.paths, sub.clone()) {
                        log::error!("update subscription: {e}");
                    }
                }
            }
            SubscriptionsMsg::DragDropSubscription(from, to) => {
                if from != to && from < self.subscriptions.len() && to < self.subscriptions.len() {
                    let sub = self.subscriptions.remove(from);
                    self.subscriptions.insert(to, sub);
                    if let Err(e) = persistence::save_subscriptions(&self.paths, &self.subscriptions) {
                        log::error!("save subscriptions: {e}");
                    }
                }
            }
            SubscriptionsMsg::DragDropNode(sub_id, from, to) => {
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == sub_id) {
                    if from != to && from < sub.nodes.len() && to < sub.nodes.len() {
                        let node = sub.nodes.remove(from);
                        sub.nodes.insert(to, node);
                        if let Err(e) = persistence::update_subscription(&self.paths, sub.clone()) {
                            log::error!("update subscription: {e}");
                        }
                    }
                }
            }
            SubscriptionsMsg::SetLocked(locked) => {
                self.locked = locked;
            }
            SubscriptionsMsg::CheckAutoUpdate => {
                let svc = self.service.clone();
                let interval = self.auto_update_interval_secs;
                sender.oneshot_command(async move {
                    let results = svc.refresh_all_overdue(interval).await;
                    let mapped: Vec<_> = results
                        .into_iter()
                        .map(|(id, r)| (id, r.map_err(|e| e.to_string())))
                        .collect();
                    SubscriptionsCmdOutput::AutoUpdateDone(mapped)
                });
                return;
            }
        }
        emit_active_nodes(&self.subscriptions, &sender);
        let expanded = capture_expanded(&self.list_container);
        render_list(&self.subscriptions, &self.list_container, &sender, &expanded, &self.testing_latency, self.locked);
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            SubscriptionsCmdOutput::RefreshDone(id, sub, result) => {
                if let Some(existing) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    *existing = sub;
                }
                log::info!(
                    "updated subscription {id}: +{} -{} ={}",
                    result.added, result.removed, result.unchanged
                );
            }
            SubscriptionsCmdOutput::LatencyResult(id, results) => {
                self.testing_latency.remove(&id);
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    for (node, latency) in sub.nodes.iter_mut().zip(results.iter()) {
                        node.last_latency_ms = *latency;
                    }
                }
            }
            SubscriptionsCmdOutput::RefreshFailed(id, error) => {
                log::error!("failed to update subscription {id}: {error}");
            }
            SubscriptionsCmdOutput::AutoUpdateDone(results) => {
                if !results.is_empty() {
                    self.subscriptions =
                        persistence::load_subscriptions(&self.paths).unwrap_or_default();
                    for (id, result) in &results {
                        match result {
                            Ok(r) => log::info!(
                                "auto-updated {id}: +{} -{} ={}",
                                r.added, r.removed, r.unchanged
                            ),
                            Err(e) => log::warn!("auto-update {id} failed: {e}"),
                        }
                    }
                }
            }
        }
        let has_active = self.subscriptions.iter().any(|s| s.has_enabled_nodes());
        let _ = sender.output(SubscriptionsOutput::ActiveNodesChanged(has_active));
        let expanded = capture_expanded(&self.list_container);
        render_list(&self.subscriptions, &self.list_container, &sender, &expanded, &self.testing_latency, self.locked);
    }
}

fn capture_expanded(container: &gtk::ListBox) -> HashSet<Uuid> {
    let mut set = HashSet::new();
    let mut child = container.first_child();
    while let Some(ref widget) = child {
        if let Some(expander) = widget.downcast_ref::<adw::ExpanderRow>() {
            if expander.is_expanded() {
                let name = expander.widget_name();
                if let Ok(id) = Uuid::parse_str(&name) {
                    set.insert(id);
                }
            }
        }
        child = widget.next_sibling();
    }
    set
}

fn render_list(
    subs: &[Subscription],
    container: &gtk::ListBox,
    sender: &ComponentSender<SubscriptionsPage>,
    expanded_subs: &HashSet<Uuid>,
    testing_latency: &HashSet<Uuid>,
    locked: bool,
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
        let expander = build_subscription_group(sub, idx, sender, expanded_subs, testing_latency, locked);
        container.append(&expander);
    }
}

fn build_subscription_group(
    sub: &Subscription,
    sub_idx: usize,
    sender: &ComponentSender<SubscriptionsPage>,
    expanded_subs: &HashSet<Uuid>,
    testing_latency: &HashSet<Uuid>,
    locked: bool,
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
        .subtitle(&format!(
            "{} | {} nodes | {}",
            source_text,
            sub.nodes.len(),
            updated_text
        ))
        .show_enable_switch(false)
        .enable_expansion(true)
        .expanded(expanded_subs.contains(&sub.id))
        .build();

    expander.set_widget_name(&sub.id.to_string());
    if !sub.enabled {
        expander.set_opacity(0.5);
    }

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
                Some(gdk::ContentProvider::for_value(&format!("sub_{idx}").to_value()))
            });
        }
        handle.add_controller(drag_source);

        let drop_target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        {
            let target_idx = sub_idx;
            let s = sender.clone();
            drop_target.connect_drop(move |_target, value, _x, _y| {
                if let Ok(val) = value.get::<String>() {
                    if let Some(from_str) = val.strip_prefix("sub_") {
                        if let Ok(from_idx) = from_str.parse::<usize>() {
                            s.input(SubscriptionsMsg::DragDropSubscription(from_idx, target_idx));
                            return true;
                        }
                    }
                }
                false
            });
        }
        expander.add_controller(drop_target);
    }
    expander.add_prefix(&handle);

    let toggle = gtk::Switch::builder()
        .active(sub.enabled)
        .valign(gtk::Align::Center)
        .sensitive(!locked)
        .build();
    {
        let id = sub.id;
        let s = sender.clone();
        toggle.connect_active_notify(move |_| {
            s.input(SubscriptionsMsg::ToggleSubscription(id));
        });
    }
    expander.add_suffix(&toggle);

    let menu_btn = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .valign(gtk::Align::Center)
        .has_frame(false)
        .sensitive(!locked)
        .build();
    menu_btn.add_css_class("flat");

    let popover = gtk::Popover::new();
    let popover_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();

    let update_btn = gtk::Button::builder()
        .label("Update")
        .has_frame(false)
        .build();
    {
        let id = sub.id;
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
        let id = sub.id;
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
        let id = sub.id;
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
        let id = sub.id;
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
        let id = sub.id;
        let s = sender.clone();
        let p = popover.clone();
        move_down_btn.connect_clicked(move |_| {
            p.popdown();
            s.input(SubscriptionsMsg::MoveSubscription(id, Direction::Down));
        });
    }

    let is_testing = testing_latency.contains(&sub.id);
    let has_latency = sub.nodes.iter().any(|n| n.last_latency_ms.is_some());

    let test_latency_btn = gtk::Button::builder()
        .label(if is_testing { "Testing..." } else { "Test Latency" })
        .has_frame(false)
        .sensitive(!is_testing)
        .build();
    {
        let id = sub.id;
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
        let id = sub.id;
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
        let id = sub.id;
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
        let id = sub.id;
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

    expander.add_suffix(&menu_btn);

    for (idx, node) in sub.nodes.iter().enumerate() {
        let node_row = build_node_row(sub.id, idx, node, sender, locked);
        expander.add_row(&node_row);
    }

    expander
}

fn build_node_row(
    sub_id: Uuid,
    idx: usize,
    node: &v2ray_rs_core::models::SubscriptionNode,
    sender: &ComponentSender<SubscriptionsPage>,
    locked: bool,
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
                Some(gdk::ContentProvider::for_value(&format!("node_{id}_{source_idx}").to_value()))
            });
        }
        node_handle.add_controller(drag_source);

        let drop_target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        {
            let target_id = sub_id;
            let target_idx = idx;
            let s = sender.clone();
            drop_target.connect_drop(move |_target, value, _x, _y| {
                if let Ok(val) = value.get::<String>() {
                    let prefix = format!("node_{target_id}_");
                    if let Some(from_str) = val.strip_prefix(&prefix) {
                        if let Ok(from_idx) = from_str.parse::<usize>() {
                            s.input(SubscriptionsMsg::DragDropNode(target_id, from_idx, target_idx));
                            return true;
                        }
                    }
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

    if let Some(ms) = node.last_latency_ms {
        let latency_label = gtk::Label::builder()
            .label(&format!("{ms}ms"))
            .valign(gtk::Align::Center)
            .build();
        latency_label.add_css_class("caption");
        if ms < 200 {
            latency_label.add_css_class("success");
        } else if ms < 500 {
            latency_label.add_css_class("warning");
        } else {
            latency_label.add_css_class("error");
        }
        row.add_suffix(&latency_label);
    }

    let move_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .valign(gtk::Align::Center)
        .build();

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
        let boundary = s.char_indices()
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
    content.append(&group);

    dialog.set_extra_child(Some(&content));

    dialog.connect_response(None, move |_, response| {
        if response == "add" {
            let name = name_entry.text().to_string();
            let url = url_entry.text().to_string();
            if !name.trim().is_empty() && !url.trim().is_empty() {
                sender.input(SubscriptionsMsg::AddSubscription(
                    name.trim().into(),
                    url.trim().into(),
                ));
            }
        }
    });

    dialog.present(gtk::Window::NONE);
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
                sender.input(SubscriptionsMsg::RenameSubscription(id, new_name.trim().into()));
            }
        }
    });

    dialog.present(gtk::Window::NONE);
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

    dialog.present(gtk::Window::NONE);
}
