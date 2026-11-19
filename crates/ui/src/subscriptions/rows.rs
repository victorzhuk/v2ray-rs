use super::*;
pub(super) fn refresh_node_widgets(
    node_widgets: &HashMap<(Uuid, Uuid), NodeWidgets>,
    subscriptions: &[Subscription],
    egress_samples: &HashMap<(Uuid, Uuid), EgressSample>,
) {
    for (&(sub_id, node_id), widgets) in node_widgets {
        let Some(node) = subscriptions
            .iter()
            .find(|sub| sub.id == sub_id)
            .and_then(|sub| sub.nodes.iter().find(|node| node.id == node_id))
        else {
            continue;
        };

        if widgets.toggle.is_active() != node.enabled {
            widgets.toggle.set_active(node.enabled);
        }
        widgets
            .row
            .set_opacity(if node.enabled { 1.0 } else { 0.5 });
        widgets.row.set_subtitle(&format_node_subtitle(
            &widgets.address,
            egress_samples.get(&(sub_id, node_id)),
        ));
        update_latency_label(&widgets.latency, node.last_latency_ms);
        update_real_delay_label(&widgets.real_delay, node.last_real_delay_ms);
    }
}

pub(super) fn update_latency_label(label: &gtk::Label, latency_ms: Option<u64>) {
    label.remove_css_class("success");
    label.remove_css_class("warning");
    label.remove_css_class("error");
    let Some(ms) = latency_ms else {
        label.set_visible(false);
        return;
    };
    label.set_label(&format!("{ms}ms"));
    label.add_css_class(latency_css_class(ms));
    label.set_visible(true);
}

pub(super) fn update_real_delay_label(label: &gtk::Label, latency_ms: Option<u64>) {
    label.remove_css_class("success");
    label.remove_css_class("warning");
    label.remove_css_class("error");
    let Some(ms) = latency_ms else {
        label.set_visible(false);
        return;
    };
    label.set_label(&format!("· {ms}ms"));
    label.add_css_class(latency_css_class(ms));
    label.set_visible(true);
}

pub(super) fn latency_css_class(ms: u64) -> &'static str {
    const LATENCY_GOOD_MS: u64 = 200;
    const LATENCY_WARN_MS: u64 = 500;
    if ms < LATENCY_GOOD_MS {
        "success"
    } else if ms < LATENCY_WARN_MS {
        "warning"
    } else {
        "error"
    }
}

pub(super) fn update_node_switch(
    node_widgets: &HashMap<(Uuid, Uuid), NodeWidgets>,
    sub_id: Uuid,
    node_idx: usize,
    subscriptions: &[Subscription],
) -> bool {
    let Some(sub) = subscriptions.iter().find(|s| s.id == sub_id) else {
        return false;
    };
    let Some(node) = sub.nodes.get(node_idx) else {
        return false;
    };
    let Some(widgets) = node_widgets.get(&(sub_id, node.id)) else {
        return false;
    };

    if widgets.toggle.is_active() != node.enabled {
        widgets.toggle.set_active(node.enabled);
    }
    widgets
        .row
        .set_opacity(if node.enabled { 1.0 } else { 0.5 });
    true
}

pub(super) fn find_expander_by_id(
    sub_expanders: &HashMap<Uuid, adw::ExpanderRow>,
    sub_id: Uuid,
) -> Option<adw::ExpanderRow> {
    sub_expanders.get(&sub_id).cloned()
}

pub(super) fn update_subscription_toggle(
    sub_expanders: &HashMap<Uuid, adw::ExpanderRow>,
    sub_id: Uuid,
    subscriptions: &[Subscription],
) -> bool {
    let sub = match subscriptions.iter().find(|s| s.id == sub_id) {
        Some(s) => s,
        None => return false,
    };
    let Some(expander) = find_expander_by_id(sub_expanders, sub_id) else {
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

pub(super) fn update_subscription_title(
    sub_expanders: &HashMap<Uuid, adw::ExpanderRow>,
    sub_id: Uuid,
    subscriptions: &[Subscription],
) -> bool {
    let sub = match subscriptions.iter().find(|s| s.id == sub_id) {
        Some(s) => s,
        None => return false,
    };
    let Some(expander) = find_expander_by_id(sub_expanders, sub_id) else {
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

pub(super) fn render_list(
    subs: &[Subscription],
    container: &gtk::ListBox,
    sender: &ComponentSender<SubscriptionsPage>,
    sub_expanders: &mut HashMap<Uuid, adw::ExpanderRow>,
    node_widgets: &mut HashMap<(Uuid, Uuid), NodeWidgets>,
    state: RenderState<'_>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    sub_expanders.clear();
    node_widgets.clear();

    if subs.is_empty() {
        let empty = adw::StatusPage::builder()
            .icon_name("folder-download-symbolic")
            .title(tr("No Subscriptions"))
            .description(tr("Add a subscription to get started"))
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
        let expander =
            build_subscription_group(sub, idx, sender, &state, sub_expanders, node_widgets);
        container.append(&expander);
    }
}

pub(super) fn build_subscription_group(
    sub: &Subscription,
    sub_idx: usize,
    sender: &ComponentSender<SubscriptionsPage>,
    state: &RenderState<'_>,
    sub_expanders: &mut HashMap<Uuid, adw::ExpanderRow>,
    node_widgets: &mut HashMap<(Uuid, Uuid), NodeWidgets>,
) -> adw::ExpanderRow {
    let expander = build_expander_header(sub, state.expanded_subs.contains(&sub.id), sender);
    attach_drag_handle(&expander, sub_idx, sender, state.locked);
    attach_subscription_toggle(&expander, sub.id, sub.enabled, sender, state.locked);
    let menu = build_subscription_menu(sub, sender, state);
    expander.add_suffix(&menu);
    sub_expanders.insert(sub.id, expander.clone());

    for (idx, node) in sub.nodes.iter().enumerate() {
        let active = state.active_node == Some((sub.id, node.id));
        let egress = state.egress_samples.get(&(sub.id, node.id));
        let (row, toggle, latency, real_delay, address) =
            build_node_row(sub.id, idx, node, sender, state.locked, active, egress);
        expander.add_row(&row);
        node_widgets.insert(
            (sub.id, node.id),
            NodeWidgets {
                row,
                toggle,
                address,
                latency,
                real_delay,
            },
        );
    }

    attach_profile_toggle(&expander, sub, sender, state.locked);

    expander
}

pub(super) fn build_expander_header(
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

pub(super) fn attach_drag_handle(
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

pub(super) fn attach_subscription_toggle(
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

pub(super) fn attach_profile_toggle(
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
        .title("Use Provider Routing &amp; DNS")
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

pub(super) fn build_subscription_menu(
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
        .sensitive(!is_testing_real_delay && real_delay_available && btn_sensitive && !tun_active)
        .build();
    if let Some(tooltip) = btn_tooltip {
        test_real_delay_btn.set_tooltip_text(Some(&tooltip));
    } else if tun_active {
        test_real_delay_btn.set_tooltip_text(Some(
            "Real Delay is unavailable while TUN is active; Disconnect first",
        ));
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

pub(super) fn build_node_row(
    sub_id: Uuid,
    idx: usize,
    node: &v2ray_rs_core::models::SubscriptionNode,
    sender: &ComponentSender<SubscriptionsPage>,
    locked: bool,
    active: bool,
    egress: Option<&EgressSample>,
) -> (adw::ActionRow, gtk::Switch, gtk::Label, gtk::Label, String) {
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

    let latency_label = gtk::Label::builder()
        .valign(gtk::Align::Center)
        .css_classes(["caption"])
        .visible(false)
        .build();
    update_latency_label(&latency_label, node.last_latency_ms);
    row.add_suffix(&latency_label);

    let real_label = gtk::Label::builder()
        .valign(gtk::Align::Center)
        .css_classes(["caption"])
        .visible(false)
        .build();
    update_real_delay_label(&real_label, node.last_real_delay_ms);
    row.add_suffix(&real_label);

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

    (row, node_toggle, latency_label, real_label, address)
}

pub(super) fn truncate(s: &str, max: usize) -> String {
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
pub(super) fn format_node_subtitle(address: &str, egress: Option<&EgressSample>) -> String {
    match egress {
        Some(sample) => format!("{address} · exit: {} · {}", sample.location, sample.address),
        None => address.to_string(),
    }
}

/// Inline hint shown next to a blocked verdict, so the rejected exit is
/// visible instead of inferable.
pub(super) fn format_blocked_hint(egress: &EgressSample) -> String {
    format!("exit: {} · {}", egress.location, egress.address)
}

/// Badge styling for a verdict: available and blocked are colored, unknown is
/// styled distinct from both.
pub(super) fn verdict_css_classes(verdict: AvailabilityVerdict) -> &'static [&'static str] {
    match verdict {
        AvailabilityVerdict::Available => &["caption", "success"],
        AvailabilityVerdict::Blocked => &["caption", "error"],
        AvailabilityVerdict::Unknown => &["caption", "dim-label"],
    }
}
