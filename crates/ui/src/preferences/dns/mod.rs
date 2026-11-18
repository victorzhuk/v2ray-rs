use adw::prelude::*;
use ipnet::{Ipv4Net, Ipv6Net};
use relm4::adw;
use relm4::gtk;
use std::cell::{Cell, RefCell};
use std::net::IpAddr;
use std::rc::Rc;
use std::str::FromStr;

use v2ray_rs_core::config::effective_dns::{DnsSource, EffectiveDns, effective_dns};
use v2ray_rs_core::models::{
    AUTO_SPLIT_DOMESTIC_TAG, AUTO_SPLIT_REMOTE_TAG, AppSettings, BackendType, DnsProtocol, DnsRule,
    DnsRuleMatch, DnsServerConfig, DnsStrategy, HostOverride, ManualNode, RoutingRuleSet,
    Subscription, builtin_dns_presets, validate_domain_keyword,
};

use crate::i18n::tr;
use crate::workspace::WorkspaceStore;

use super::{
    RoutingObservers, SettingsCallback, SettingsObservers, clear_preferences_group, emit,
    subscribe_settings,
};

pub(super) fn build_dns_page(
    store: &WorkspaceStore,
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    settings_observers: &SettingsObservers,
    rules: &Rc<RefCell<RoutingRuleSet>>,
    routing_observers: &RoutingObservers,
    nodes_observers: &RoutingObservers,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(tr("DNS"))
        .icon_name("network-transmit-symbolic")
        .build();

    let s = state.borrow();

    let primary_group = adw::PreferencesGroup::builder()
        .title(tr("Primary DNS"))
        .build();

    let enable_row = adw::SwitchRow::builder()
        .title(tr("Enable DNS"))
        .active(s.dns.enabled)
        .build();
    primary_group.add(&enable_row);

    let strategy_row = adw::ComboRow::builder()
        .title(tr("IP Strategy"))
        .model(&gtk::StringList::new(&[
            &tr("Prefer IPv4"),
            &tr("Prefer IPv6"),
            &tr("IPv4 Only"),
            &tr("IPv6 Only"),
        ]))
        .selected(strategy_to_index(s.dns.strategy))
        .build();
    primary_group.add(&strategy_row);

    let remote_server_row = adw::ActionRow::builder()
        .title(tr("Remote"))
        .subtitle(tr("Not configured"))
        .build();
    let remote_edit_btn = gtk::Button::builder()
        .icon_name("document-edit-symbolic")
        .valign(gtk::Align::Center)
        .has_frame(false)
        .css_classes(["flat"])
        .tooltip_text(tr("Edit Remote Server"))
        .build();
    remote_server_row.add_suffix(&remote_edit_btn);
    primary_group.add(&remote_server_row);

    let domestic_server_row = adw::ActionRow::builder()
        .title(tr("Domestic"))
        .subtitle(tr("Not configured"))
        .build();
    let domestic_edit_btn = gtk::Button::builder()
        .icon_name("document-edit-symbolic")
        .valign(gtk::Align::Center)
        .has_frame(false)
        .css_classes(["flat"])
        .tooltip_text(tr("Edit Domestic Server"))
        .build();
    domestic_server_row.add_suffix(&domestic_edit_btn);
    primary_group.add(&domestic_server_row);

    page.add(&primary_group);

    drop(s);

    // GTK setters emit their notify signals synchronously, so a programmatic
    // update re-enters the handler below. Raised while the page drives its own
    // widgets; the handlers bail out instead of writing the value back.
    let suppress = Rc::new(Cell::new(false));

    {
        let st = state.clone();
        let cb = cb.clone();
        let suppress = suppress.clone();
        enable_row.connect_active_notify(move |row| {
            if suppress.get() {
                return;
            }
            st.borrow_mut().dns.enabled = row.is_active();
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        let suppress = suppress.clone();
        strategy_row.connect_selected_notify(move |row| {
            if suppress.get() {
                return;
            }
            st.borrow_mut().dns.strategy = index_to_strategy(row.selected());
            emit(&st, &cb);
        });
    }
    let advanced_expander = adw::ExpanderRow::builder()
        .title(tr("Advanced"))
        .subtitle(tr("Full configuration"))
        .build();

    let servers_group = adw::PreferencesGroup::builder()
        .title(tr("Servers"))
        .build();
    let servers_toolbar = adw::ActionRow::builder().activatable(false).build();
    let servers_toolbar_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::End)
        .hexpand(true)
        .build();
    let providers_btn = gtk::Button::builder()
        .icon_name("starred-symbolic")
        .tooltip_text("DNS Providers")
        .css_classes(["flat"])
        .build();
    servers_toolbar_box.append(&providers_btn);
    let add_server_btn = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add Server")
        .css_classes(["flat"])
        .build();
    servers_toolbar_box.append(&add_server_btn);
    servers_toolbar.add_suffix(&servers_toolbar_box);
    servers_group.add(&servers_toolbar);

    let rules_group = adw::PreferencesGroup::builder().title("DNS Rules").build();
    let rules_toolbar = adw::ActionRow::builder().activatable(false).build();
    let rules_toolbar_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::End)
        .hexpand(true)
        .build();
    let custom_rules_switch = gtk::Switch::builder()
        .active(state.borrow().dns.use_custom_rules)
        .valign(gtk::Align::Center)
        .build();
    let custom_rules_label = gtk::Label::builder()
        .label("Custom Rules")
        .margin_start(6)
        .build();
    let add_rule_btn = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add Rule")
        .css_classes(["flat"])
        .sensitive(state.borrow().dns.use_custom_rules)
        .build();
    rules_toolbar_box.append(&custom_rules_switch);
    rules_toolbar_box.append(&custom_rules_label);
    rules_toolbar_box.append(&add_rule_btn);
    rules_toolbar.add_suffix(&rules_toolbar_box);
    rules_group.add(&rules_toolbar);

    let hosts_group = adw::PreferencesGroup::builder().title("Hosts").build();
    let hosts_toolbar = adw::ActionRow::builder().activatable(false).build();
    let hosts_toolbar_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::End)
        .hexpand(true)
        .build();
    let add_host_btn = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add Host")
        .css_classes(["flat"])
        .build();
    hosts_toolbar_box.append(&add_host_btn);
    hosts_toolbar.add_suffix(&hosts_toolbar_box);
    hosts_group.add(&hosts_toolbar);

    let is_singbox = state.borrow().backend.backend_type == BackendType::SingBox;

    let fakeip_group = adw::PreferencesGroup::builder()
        .title("FakeIP")
        .visible(is_singbox)
        .build();
    let fakeip_enable_row = adw::SwitchRow::builder()
        .title("Enable FakeIP")
        .active(state.borrow().dns.fakeip.enabled)
        .build();
    fakeip_group.add(&fakeip_enable_row);

    let fakeip_inet4_row = adw::EntryRow::builder()
        .title("IPv4 Range")
        .text(&state.borrow().dns.fakeip.inet4_range)
        .build();
    let fakeip_inet4_error = gtk::Label::builder()
        .label("")
        .css_classes(["error-label"])
        .halign(gtk::Align::Start)
        .margin_start(12)
        .margin_bottom(6)
        .visible(false)
        .build();
    fakeip_group.add(&fakeip_inet4_row);
    fakeip_group.add(&fakeip_inet4_error);

    let fakeip_inet6_row = adw::EntryRow::builder()
        .title("IPv6 Range")
        .text(&state.borrow().dns.fakeip.inet6_range)
        .build();
    let fakeip_inet6_error = gtk::Label::builder()
        .label("")
        .css_classes(["error-label"])
        .halign(gtk::Align::Start)
        .margin_start(12)
        .margin_bottom(6)
        .visible(false)
        .build();
    fakeip_group.add(&fakeip_inet6_row);
    fakeip_group.add(&fakeip_inet6_error);

    let disable_cache_row = adw::SwitchRow::builder()
        .title("Disable Cache")
        .active(state.borrow().dns.disable_cache)
        .build();

    let client_subnet_row = adw::EntryRow::builder()
        .title("Client Subnet IP")
        .text(
            state
                .borrow()
                .dns
                .client_subnet
                .as_deref()
                .unwrap_or_default(),
        )
        .build();
    let client_subnet_error = gtk::Label::builder()
        .label("")
        .css_classes(["error-label"])
        .halign(gtk::Align::Start)
        .margin_start(12)
        .margin_bottom(6)
        .visible(false)
        .build();

    advanced_expander.add_row(&servers_group);
    advanced_expander.add_row(&rules_group);
    advanced_expander.add_row(&hosts_group);
    advanced_expander.add_row(&fakeip_group);
    advanced_expander.add_row(&disable_cache_row);
    advanced_expander.add_row(&client_subnet_row);
    advanced_expander.add_row(&client_subnet_error);

    let advanced_group = adw::PreferencesGroup::new();

    let effective_group = adw::PreferencesGroup::builder()
        .title(tr("Effective DNS"))
        .build();
    {
        let rules = rules.clone();
        let store = store.clone();
        let effective_group = effective_group.clone();
        let render_effective: Rc<dyn Fn(&AppSettings)> = Rc::new(move |settings: &AppSettings| {
            clear_preferences_group(&effective_group);
            let subscriptions = store.load_subscriptions().unwrap_or_default();
            let manual_nodes = store.load_manual_nodes_or_default();
            let profile_subscriptions = dns_profile_subscription_names(&subscriptions);
            let node_addrs = collect_enabled_node_addresses(&subscriptions, &manual_nodes);
            let node_addr_refs: Vec<&str> = node_addrs.iter().map(String::as_str).collect();
            let summary = effective_dns(
                settings.backend.backend_type,
                settings,
                rules.borrow().rules(),
                &node_addr_refs,
            );
            for row in effective_dns_rows(&summary, &profile_subscriptions) {
                effective_group.add(&adw::ActionRow::builder().title(row).build());
            }
        });
        render_effective(&state.borrow());
        {
            let render_effective = render_effective.clone();
            subscribe_settings(settings_observers, move |settings| {
                render_effective(settings);
            });
        }
        {
            let render_effective = render_effective.clone();
            let state = state.clone();
            routing_observers
                .borrow_mut()
                .push(Rc::new(move || render_effective(&state.borrow())));
        }
        {
            let render_effective = render_effective.clone();
            let state = state.clone();
            nodes_observers
                .borrow_mut()
                .push(Rc::new(move || render_effective(&state.borrow())));
        }
    }

    page.add(&effective_group);
    advanced_group.add(&advanced_expander);
    page.add(&advanced_group);

    {
        let st = state.clone();
        let cb = cb.clone();
        disable_cache_row.connect_active_notify(move |row| {
            st.borrow_mut().dns.disable_cache = row.is_active();
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        let error_label = client_subnet_error.clone();
        let row = client_subnet_row.clone();
        client_subnet_row.connect_changed(move |_| {
            let text = row.text().to_string();
            let trimmed = text.trim();

            match validate_ip_address(trimmed) {
                Ok(()) => {
                    let result = apply_dns_settings_mutation(&st, &cb, |settings| {
                        settings.dns.client_subnet = if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed.to_string())
                        };
                        Ok(())
                    });

                    match result {
                        Ok(()) => {
                            error_label.set_visible(false);
                            row.remove_css_class("error");
                        }
                        Err(msg) => {
                            error_label.set_text(&msg);
                            error_label.set_visible(true);
                            row.add_css_class("error");
                        }
                    }
                }
                Err(msg) => {
                    error_label.set_text(&msg);
                    error_label.set_visible(true);
                    row.add_css_class("error");
                }
            }
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        let inet4_error = fakeip_inet4_error.clone();
        let inet6_error = fakeip_inet6_error.clone();
        let inet4_row = fakeip_inet4_row.clone();
        let inet6_row = fakeip_inet6_row.clone();
        fakeip_enable_row.connect_active_notify(move |row| {
            let enabled = row.is_active();

            if enabled {
                let inet4_text = inet4_row.text().to_string();
                let inet6_text = inet6_row.text().to_string();
                let mut can_commit = true;

                if let Err(msg) = validate_ipv4_cidr(&inet4_text) {
                    inet4_error.set_text(&msg);
                    inet4_error.set_visible(true);
                    inet4_row.add_css_class("error");
                    can_commit = false;
                }

                if let Err(msg) = validate_ipv6_cidr(&inet6_text) {
                    inet6_error.set_text(&msg);
                    inet6_error.set_visible(true);
                    inet6_row.add_css_class("error");
                    can_commit = false;
                }

                if !can_commit {
                    return;
                }
            } else {
                inet4_error.set_visible(false);
                inet6_error.set_visible(false);
                inet4_row.remove_css_class("error");
                inet6_row.remove_css_class("error");
            }

            let _ = apply_dns_settings_mutation(&st, &cb, |settings| {
                settings.dns.fakeip.enabled = enabled;
                Ok(())
            });
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        let error_label = fakeip_inet4_error.clone();
        let row = fakeip_inet4_row.clone();
        let enable_row = fakeip_enable_row.clone();
        fakeip_inet4_row.connect_changed(move |_| {
            let text = row.text().to_string();

            let enabled = enable_row.is_active();
            if enabled {
                match validate_ipv4_cidr(&text) {
                    Ok(()) => {
                        match apply_dns_settings_mutation(&st, &cb, |settings| {
                            settings.dns.fakeip.inet4_range = text.clone();
                            Ok(())
                        }) {
                            Ok(()) => {
                                error_label.set_visible(false);
                                row.remove_css_class("error");
                            }
                            Err(msg) => {
                                error_label.set_text(&msg);
                                error_label.set_visible(true);
                                row.add_css_class("error");
                            }
                        }
                    }
                    Err(msg) => {
                        error_label.set_text(&msg);
                        error_label.set_visible(true);
                        row.add_css_class("error");
                    }
                }
            } else {
                error_label.set_visible(false);
                row.remove_css_class("error");
                let _ = apply_dns_settings_mutation(&st, &cb, |settings| {
                    settings.dns.fakeip.inet4_range = text.clone();
                    Ok(())
                });
            }
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        let error_label = fakeip_inet6_error.clone();
        let row = fakeip_inet6_row.clone();
        let enable_row = fakeip_enable_row.clone();
        fakeip_inet6_row.connect_changed(move |_| {
            let text = row.text().to_string();

            let enabled = enable_row.is_active();
            if enabled {
                match validate_ipv6_cidr(&text) {
                    Ok(()) => {
                        match apply_dns_settings_mutation(&st, &cb, |settings| {
                            settings.dns.fakeip.inet6_range = text.clone();
                            Ok(())
                        }) {
                            Ok(()) => {
                                error_label.set_visible(false);
                                row.remove_css_class("error");
                            }
                            Err(msg) => {
                                error_label.set_text(&msg);
                                error_label.set_visible(true);
                                row.add_css_class("error");
                            }
                        }
                    }
                    Err(msg) => {
                        error_label.set_text(&msg);
                        error_label.set_visible(true);
                        row.add_css_class("error");
                    }
                }
            } else {
                error_label.set_visible(false);
                row.remove_css_class("error");
                let _ = apply_dns_settings_mutation(&st, &cb, |settings| {
                    settings.dns.fakeip.inet6_range = text.clone();
                    Ok(())
                });
            }
        });
    }

    let dns_ctx = DnsRenderCtx {
        state: state.clone(),
        cb: cb.clone(),
        suppress: suppress.clone(),
        enable_row: enable_row.clone(),
        servers_group: servers_group.clone(),
        rules_group: rules_group.clone(),
        hosts_group: hosts_group.clone(),
        strategy_row: strategy_row.clone(),
        remote_row: remote_server_row.clone(),
        domestic_row: domestic_server_row.clone(),
        remote_edit_btn: remote_edit_btn.clone(),
        domestic_edit_btn: domestic_edit_btn.clone(),
        added_servers: Rc::new(RefCell::new(Vec::new())),
        added_rules: Rc::new(RefCell::new(Vec::new())),
        added_hosts: Rc::new(RefCell::new(Vec::new())),
    };

    render_dns_servers(&dns_ctx);
    render_dns_rules(&dns_ctx);
    render_dns_hosts(&dns_ctx);
    render_primary_dns_servers(&dns_ctx);
    {
        let sync_dns_ui: Rc<dyn Fn(&AppSettings)> = Rc::new({
            let strategy_row = strategy_row.clone();
            let remote_server_row = remote_server_row.clone();
            let domestic_server_row = domestic_server_row.clone();
            let advanced_expander = advanced_expander.clone();
            let servers_group = servers_group.clone();
            let rules_group = rules_group.clone();
            let hosts_group = hosts_group.clone();
            let fakeip_group = fakeip_group.clone();
            let disable_cache_row = disable_cache_row.clone();
            let client_subnet_row = client_subnet_row.clone();
            move |settings| {
                let dns_enabled = settings.dns.enabled;
                strategy_row.set_sensitive(dns_enabled);
                match strategy_family_note(settings.backend.backend_type) {
                    Some(note) => strategy_row.set_subtitle(note),
                    None => strategy_row.set_subtitle(""),
                }
                remote_server_row.set_sensitive(dns_enabled);
                domestic_server_row.set_sensitive(dns_enabled);
                advanced_expander.set_sensitive(dns_enabled);
                servers_group.set_sensitive(dns_enabled);
                rules_group.set_sensitive(dns_enabled);
                hosts_group.set_sensitive(dns_enabled);
                fakeip_group.set_sensitive(dns_enabled);
                fakeip_group.set_visible(settings.backend.backend_type == BackendType::SingBox);
                disable_cache_row.set_sensitive(dns_enabled);
                client_subnet_row.set_sensitive(dns_enabled);
            }
        });
        sync_dns_ui(&state.borrow());

        let sync_dns_ui_observer = sync_dns_ui.clone();
        let ctx = dns_ctx.clone();
        let last_render_inputs = Rc::new(RefCell::new((
            ctx.state.borrow().backend.backend_type,
            ctx.state.borrow().tun.enabled,
        )));
        subscribe_settings(settings_observers, move |settings| {
            let backend = settings.backend.backend_type;
            sync_dns_ui_observer(settings);
            let render_inputs = (backend, settings.tun.enabled);
            if render_inputs != *last_render_inputs.borrow() {
                *last_render_inputs.borrow_mut() = render_inputs;
                render_dns_servers(&ctx);
                render_primary_dns_servers(&ctx);
            }
        });
    }

    {
        let ctx = dns_ctx.clone();
        let add_rule_btn = add_rule_btn.clone();
        custom_rules_switch.connect_active_notify(move |sw| {
            let active = sw.is_active();
            if apply_dns_mutation(&ctx, |settings| {
                settings.dns.use_custom_rules = active;
                Ok(())
            })
            .is_ok()
            {
                add_rule_btn.set_sensitive(active);
                render_dns_rules(&ctx);
            }
        });
    }
    {
        let ctx = dns_ctx.clone();
        remote_edit_btn.connect_clicked(move |_| {
            let server = ctx
                .state
                .borrow()
                .dns
                .servers
                .iter()
                .find(|server| server.tag == "remote")
                .cloned();
            if let Some(server) = server {
                show_dns_server_dialog(Some(server), &ctx);
            }
        });
    }
    {
        let ctx = dns_ctx.clone();
        domestic_edit_btn.connect_clicked(move |_| {
            let server = ctx
                .state
                .borrow()
                .dns
                .servers
                .iter()
                .find(|server| server.tag == "domestic")
                .cloned();
            if let Some(server) = server {
                show_dns_server_dialog(Some(server), &ctx);
            }
        });
    }
    {
        let ctx = dns_ctx.clone();
        providers_btn.connect_clicked(move |_| {
            show_dns_providers_dialog(&ctx);
        });
    }
    {
        let ctx = dns_ctx.clone();
        add_server_btn.connect_clicked(move |_| {
            show_dns_server_dialog(None, &ctx);
        });
    }
    {
        let ctx = dns_ctx.clone();
        add_rule_btn.connect_clicked(move |_| {
            show_dns_rule_dialog(None, &ctx);
        });
    }
    {
        let ctx = dns_ctx.clone();
        add_host_btn.connect_clicked(move |_| {
            show_dns_host_dialog(None, &ctx);
        });
    }

    page
}

mod effective;
mod hosts;
mod rules;
mod servers;
use effective::*;
use hosts::*;
use rules::*;
use servers::*;

#[derive(Clone)]
struct DnsRenderCtx {
    state: Rc<RefCell<AppSettings>>,
    cb: SettingsCallback,
    suppress: Rc<Cell<bool>>,
    enable_row: adw::SwitchRow,
    servers_group: adw::PreferencesGroup,
    rules_group: adw::PreferencesGroup,
    hosts_group: adw::PreferencesGroup,
    strategy_row: adw::ComboRow,
    remote_row: adw::ActionRow,
    domestic_row: adw::ActionRow,
    remote_edit_btn: gtk::Button,
    domestic_edit_btn: gtk::Button,
    added_servers: Rc<RefCell<Vec<adw::ActionRow>>>,
    added_rules: Rc<RefCell<Vec<adw::ActionRow>>>,
    added_hosts: Rc<RefCell<Vec<adw::ActionRow>>>,
}

/// Drives widgets whose handlers write back into the shared settings state.
fn without_handlers(suppress: &Cell<bool>, update: impl FnOnce()) {
    suppress.set(true);
    update();
    suppress.set(false);
}

fn strategy_to_index(s: DnsStrategy) -> u32 {
    match s {
        DnsStrategy::PreferIpv4 => 0,
        DnsStrategy::PreferIpv6 => 1,
        DnsStrategy::Ipv4Only => 2,
        DnsStrategy::Ipv6Only => 3,
    }
}

fn index_to_strategy(i: u32) -> DnsStrategy {
    match i {
        1 => DnsStrategy::PreferIpv6,
        2 => DnsStrategy::Ipv4Only,
        3 => DnsStrategy::Ipv6Only,
        _ => DnsStrategy::PreferIpv4,
    }
}

fn validate_ipv4_cidr(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("IPv4 CIDR is required".to_string());
    }
    Ipv4Net::from_str(value.trim())
        .map(|_| ())
        .map_err(|_| "Invalid IPv4 CIDR notation".to_string())
}

fn validate_ipv6_cidr(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("IPv6 CIDR is required".to_string());
    }
    Ipv6Net::from_str(value.trim())
        .map(|_| ())
        .map_err(|_| "Invalid IPv6 CIDR notation".to_string())
}

fn validate_ip_address(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    IpAddr::from_str(trimmed)
        .map(|_| ())
        .map_err(|_| "Invalid IP address".to_string())
}

fn protocol_to_index(p: DnsProtocol) -> u32 {
    match p {
        DnsProtocol::Udp => 0,
        DnsProtocol::Tcp => 1,
        DnsProtocol::Doh => 2,
        DnsProtocol::Dot => 3,
        DnsProtocol::Doq => 4,
        DnsProtocol::H3 => 5,
    }
}

fn index_to_protocol(i: u32) -> DnsProtocol {
    match i {
        1 => DnsProtocol::Tcp,
        2 => DnsProtocol::Doh,
        3 => DnsProtocol::Dot,
        4 => DnsProtocol::Doq,
        5 => DnsProtocol::H3,
        _ => DnsProtocol::Udp,
    }
}

fn protocol_display_name(p: DnsProtocol) -> &'static str {
    match p {
        DnsProtocol::Udp => "UDP",
        DnsProtocol::Tcp => "TCP",
        DnsProtocol::Doh => "DoH",
        DnsProtocol::Dot => "DoT",
        DnsProtocol::Doq => "DoQ",
        DnsProtocol::H3 => "H3",
    }
}

const PRIVATE_DNS_WARNING: &str = "Queries to this private address go through the proxy and reach the proxy server's network, not yours. Set Detour to 'direct' if this resolver is on your network.";

fn apply_dns_settings_mutation<F>(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    mutate: F,
) -> Result<(), String>
where
    F: FnOnce(&mut AppSettings) -> Result<(), String>,
{
    let mut next = state.borrow().clone();
    mutate(&mut next)?;
    validate_dns_settings_for_backend(&next)?;

    {
        let mut current = state.borrow_mut();
        *current = next.clone();
    }

    cb(next);
    Ok(())
}

fn apply_dns_mutation<F>(ctx: &DnsRenderCtx, mutate: F) -> Result<(), String>
where
    F: FnOnce(&mut AppSettings) -> Result<(), String>,
{
    apply_dns_settings_mutation(&ctx.state, &ctx.cb, mutate)
}

fn set_validation_message(label: &gtk::Label, message: Option<&str>) {
    if let Some(message) = message {
        label.set_text(message);
        label.set_visible(true);
    } else {
        label.set_text("");
        label.set_visible(false);
    }
}

fn render_primary_dns_servers(ctx: &DnsRenderCtx) {
    let servers = ctx.state.borrow().dns.servers.clone();
    let backend = ctx.state.borrow().backend.backend_type;
    let tun_enabled = ctx.state.borrow().tun.enabled;
    let use_custom_rules = ctx.state.borrow().dns.use_custom_rules;

    let remote_server = servers.iter().find(|s| s.tag == "remote");
    let domestic_server = servers.iter().find(|s| s.tag == "domestic");

    if let Some(server) = remote_server {
        ctx.remote_row
            .set_subtitle(&primary_dns_subtitle(server, backend, tun_enabled));
        ctx.remote_edit_btn.set_sensitive(true);
    } else {
        ctx.remote_row.set_subtitle(primary_row_missing_note(
            AUTO_SPLIT_REMOTE_TAG,
            use_custom_rules,
        ));
        ctx.remote_edit_btn.set_sensitive(false);
    }

    if let Some(server) = domestic_server {
        ctx.domestic_row
            .set_subtitle(&primary_dns_subtitle(server, backend, tun_enabled));
        ctx.domestic_edit_btn.set_sensitive(true);
    } else {
        ctx.domestic_row.set_subtitle(primary_row_missing_note(
            AUTO_SPLIT_DOMESTIC_TAG,
            use_custom_rules,
        ));
        ctx.domestic_edit_btn.set_sensitive(false);
    }
}

fn show_dns_providers_dialog(ctx: &DnsRenderCtx) {
    let dialog = adw::AlertDialog::builder().heading("DNS Providers").build();
    dialog.add_response("close", "Close");
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();

    let builtin_group = adw::PreferencesGroup::builder().title("Built-in").build();
    for preset in builtin_dns_presets() {
        let row = adw::ActionRow::builder()
            .title(&preset.name)
            .subtitle(&preset.description)
            .build();
        let apply_btn = gtk::Button::builder()
            .label("Apply")
            .valign(gtk::Align::Center)
            .css_classes(["suggested-action"])
            .build();
        let ctx = ctx.clone();
        let p = preset.clone();
        let providers_dialog = dialog.clone();
        apply_btn.connect_clicked(move |_| {
            let confirm_dialog = adw::AlertDialog::builder()
                .heading("Apply DNS Provider")
                .body(format!("Replace current DNS servers with {}?", p.name))
                .build();
            confirm_dialog.add_response("cancel", "Cancel");
            confirm_dialog.add_response("apply", "Apply");
            confirm_dialog.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
            confirm_dialog.set_default_response(Some("cancel"));
            confirm_dialog.set_close_response("cancel");

            let ctx_inner = ctx.clone();
            let p_inner = p.clone();
            let pd = providers_dialog.clone();
            confirm_dialog.connect_response(None, move |_, response| {
                if response != "apply" {
                    return;
                }

                if apply_dns_mutation(&ctx_inner, |settings| {
                    settings.dns.apply_dns_preset(&p_inner);
                    Ok(())
                })
                .is_ok()
                {
                    render_dns_servers(&ctx_inner);
                    render_dns_rules(&ctx_inner);
                    render_primary_dns_servers(&ctx_inner);
                    let (enabled, strategy) = {
                        let s = ctx_inner.state.borrow();
                        (s.dns.enabled, strategy_to_index(s.dns.strategy))
                    };
                    without_handlers(&ctx_inner.suppress, || {
                        ctx_inner.enable_row.set_active(enabled);
                        ctx_inner.strategy_row.set_selected(strategy);
                    });
                    pd.close();
                }
            });

            confirm_dialog.present(crate::active_window().as_ref());
        });
        row.add_suffix(&apply_btn);
        builtin_group.add(&row);
    }
    content.append(&builtin_group);

    let scrolled = gtk::ScrolledWindow::builder()
        .min_content_height(300)
        .max_content_height(500)
        .child(&content)
        .build();

    dialog.set_extra_child(Some(&scrolled));
    dialog.present(crate::active_window().as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use v2ray_rs_core::models::{DnsConfig, ImportedProfile};

    /// The crash this guard exists for: `set_selected` emits `notify::selected`
    /// synchronously, so a handler holding the shared state would re-enter and
    /// panic. GTK is needed for the real signal emission, so skip where there
    /// is no display to initialize against.
    fn effective_dns_rows_xray_tun(dns_enabled: bool) -> EffectiveDns {
        let mut settings = AppSettings::default();
        settings.tun.enabled = true;
        settings.dns.enabled = dns_enabled;
        effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"])
    }

    #[test]
    fn effective_dns_rows_lists_fallback_pair_when_dns_off() {
        let rows = effective_dns_rows(&effective_dns_rows_xray_tun(false), &[]);
        assert_eq!(rows.len(), 2, "both fallback DoH resolvers, one row each");
        assert!(
            rows[0].starts_with(
                "https://1.1.1.1/dns-query [DoH] path=proxy source=fallback scope=all"
            ),
            "got: {}",
            rows[0]
        );
        assert!(
            rows[1].starts_with(
                "https://8.8.8.8/dns-query [DoH] path=proxy source=fallback scope=all"
            ),
            "got: {}",
            rows[1]
        );
    }

    #[test]
    fn effective_dns_rows_follow_enabled_dns() {
        let mut settings = AppSettings::default();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "main".into(),
            protocol: DnsProtocol::Udp,
            address: "8.8.8.8".into(),
            port: None,
            detour: None,
        }];
        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);
        let rows = effective_dns_rows(&summary, &[]);
        assert!(
            rows.iter().all(|row| !row.contains("source=fallback")),
            "configured servers must replace the fallback rows: {rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.starts_with("8.8.8.8:53 [UDP]")
                && row.contains("source=user")
                && row.ends_with("scope=all")),
            "got: {rows:?}"
        );
    }

    #[test]
    fn effective_dns_rows_names_imported_profile_subscription() {
        let mut sub = Subscription::new_from_url("Provider", "https://example.com/sub");
        sub.use_imported_profile = true;
        sub.imported_profile = Some(ImportedProfile {
            rules: Vec::new(),
            dns: Some(DnsConfig::default()),
            skipped: Vec::new(),
            imported_at: Utc::now(),
        });
        let mut without_dns = Subscription::new_from_url("Plain", "https://example.com/plain");
        without_dns.enabled = false;

        assert_eq!(
            dns_profile_subscription_names(&[sub, without_dns]),
            vec!["Provider".to_string()],
            "only the enabled DNS-carrying profile subscription is listed"
        );

        let rows = effective_dns_rows(&effective_dns_rows_xray_tun(true), &["Provider".into()]);
        assert!(
            rows.contains(&"nodes from \"Provider\" use the imported profile's DNS instead".into()),
            "got: {rows:?}"
        );
    }

    #[test]
    fn effective_dns_rows_collapses_bootstrap_pair() {
        let mut settings = AppSettings::default();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "main".into(),
            protocol: DnsProtocol::Udp,
            address: "dns.example.com".into(),
            port: None,
            detour: None,
        }];
        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);
        assert!(
            summary
                .entries()
                .iter()
                .filter(|entry| entry.source() == DnsSource::Bootstrap)
                .count()
                >= 2,
            "fixture must produce the consecutive bootstrap pair"
        );
        let rows = effective_dns_rows(&summary, &[]);
        assert_eq!(
            rows.iter()
                .filter(|row| *row == "bootstrap resolvers for proxy hostnames (xray TUN)")
                .count(),
            1,
            "the bootstrap pair collapses to one row: {rows:?}"
        );
    }

    #[test]
    fn effective_dns_rows_no_profile_row_without_profiles() {
        let rows = effective_dns_rows(&effective_dns_rows_xray_tun(true), &[]);
        assert!(
            rows.iter()
                .all(|row| !row.contains("use the imported profile's DNS")),
            "no profile subscription, no profile row: {rows:?}"
        );
    }

    #[test]
    fn programmatic_selection_does_not_re_enter_the_handler() {
        crate::gtk_test::run(|| {
            let state = Rc::new(RefCell::new(AppSettings::default()));
            let suppress = Rc::new(Cell::new(false));
            let writes = Rc::new(Cell::new(0u32));

            let row = adw::ComboRow::builder()
                .model(&gtk::StringList::new(&[
                    "Prefer IPv4",
                    "Prefer IPv6",
                    "IPv4 Only",
                    "IPv6 Only",
                ]))
                .selected(strategy_to_index(DnsStrategy::Ipv4Only))
                .build();

            {
                let state = state.clone();
                let suppress = suppress.clone();
                let writes = writes.clone();
                row.connect_selected_notify(move |row| {
                    if suppress.get() {
                        return;
                    }
                    // Would panic if a borrow were still alive from the caller.
                    state.borrow_mut().dns.strategy = index_to_strategy(row.selected());
                    writes.set(writes.get() + 1);
                });
            }

            state.borrow_mut().dns.strategy = DnsStrategy::PreferIpv4;
            // Deliberately the shape that used to abort: a live borrow across the
            // setter. Safe only because the handler bails out before taking one.
            without_handlers(&suppress, || {
                row.set_selected(strategy_to_index(state.borrow().dns.strategy))
            });

            assert_eq!(row.selected(), strategy_to_index(DnsStrategy::PreferIpv4));
            assert_eq!(writes.get(), 0, "the handler must not write the value back");
            assert!(!suppress.get(), "the guard must be released");
        });
    }

    #[test]
    fn test_strategy_family_note_per_backend() {
        for backend in [BackendType::Xray, BackendType::V2ray] {
            let note = strategy_family_note(backend)
                .unwrap_or_else(|| panic!("{backend:?} must show the strategy note"));
            assert_eq!(
                note,
                "Prefer IPv4 and Prefer IPv6 query only the preferred address family on this backend",
                "{backend:?} note mismatch"
            );
        }
        assert_eq!(
            strategy_family_note(BackendType::SingBox),
            None,
            "sing-box has native family handling, no note"
        );
    }

    #[test]
    fn detour_note_matches_backend_detour_capabilities() {
        let singbox = detour_note(BackendType::SingBox).expect("sing-box honors detour");
        assert!(
            singbox.contains("proxy"),
            "sing-box: proxy detour explained"
        );
        assert!(
            singbox.contains("direct"),
            "sing-box: direct detour explained"
        );

        let xray = detour_note(BackendType::Xray).expect("xray has a direct-only note");
        assert!(xray.contains("TUN"), "xray detour only applies under TUN");

        assert_eq!(
            detour_note(BackendType::V2ray),
            None,
            "v2ray has neither mechanism, so the row is hidden with no subtitle"
        );
    }

    fn dns_rule_with_keyword(keyword: &str) -> DnsRuleMatch {
        DnsRuleMatch::DomainKeyword {
            keyword: keyword.to_string(),
        }
    }

    #[test]
    fn dns_rule_match_validation_error_rejects_wildcard_and_whitespace() {
        let msg = dns_rule_match_validation_error(&dns_rule_with_keyword("*.ru"))
            .expect("wildcard keyword must be rejected");
        assert!(msg.contains("plain substring"), "{msg}");
        assert!(msg.contains("Domain rule type"), "{msg}");

        assert!(dns_rule_match_validation_error(&dns_rule_with_keyword("has space")).is_some());
    }

    #[test]
    fn dns_rule_match_validation_error_passes_plain_keyword_and_other_matches() {
        assert_eq!(
            dns_rule_match_validation_error(&dns_rule_with_keyword("sina")),
            None
        );
        for m in [
            DnsRuleMatch::DomainSuffix {
                suffix: ".ru".to_string(),
            },
            DnsRuleMatch::GeoSite {
                category: "google".to_string(),
            },
            DnsRuleMatch::DomainFull {
                domain: "example.com".to_string(),
            },
        ] {
            assert_eq!(dns_rule_match_validation_error(&m), None, "{m:?}");
        }
    }

    fn flagged_server(address: &str, detour: Option<&str>) -> DnsServerConfig {
        DnsServerConfig {
            tag: "domestic".into(),
            protocol: DnsProtocol::Udp,
            address: address.into(),
            port: Some(53),
            detour: detour.map(Into::into),
        }
    }

    #[test]
    fn test_private_dns_warning() {
        let warning = private_dns_warning(
            &flagged_server("127.0.0.1", Some("proxy")),
            BackendType::SingBox,
            true,
        )
        .expect("private IP on the proxy detour is flagged");
        assert_eq!(warning, PRIVATE_DNS_WARNING);
        assert!(warning.contains("proxy server's network"), "{warning}");
        assert!(warning.contains("direct"), "{warning}");

        for unflagged in [
            flagged_server("127.0.0.1", Some("direct")),
            flagged_server("1.1.1.1", Some("proxy")),
            flagged_server("dns.google", Some("proxy")),
        ] {
            assert_eq!(
                private_dns_warning(&unflagged, BackendType::SingBox, true),
                None,
                "{:?}",
                unflagged
            );
        }
        assert_eq!(
            private_dns_warning(
                &flagged_server("127.0.0.1", Some("proxy")),
                BackendType::V2ray,
                true
            ),
            None,
            "v2ray is never flagged"
        );
    }

    #[test]
    fn test_flagged_private_server_still_validates() {
        let mut settings = AppSettings::default();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = true;
        settings.dns.servers = vec![flagged_server("192.168.1.1", Some("proxy"))];

        assert!(
            validate_dns_settings_for_backend(&settings).is_ok(),
            "a flagged private server is a warning, not a validation error"
        );
    }

    #[test]
    fn test_server_row_subtitle_appends_private_warning() {
        let flagged = flagged_server("127.0.0.1", Some("proxy"));
        let subtitle = server_row_subtitle(&flagged, BackendType::SingBox, true);
        assert!(subtitle.starts_with("udp://127.0.0.1:53"), "{subtitle}");
        assert!(subtitle.contains(PRIVATE_DNS_WARNING), "{subtitle}");

        let direct = flagged_server("127.0.0.1", Some("direct"));
        let subtitle = server_row_subtitle(&direct, BackendType::SingBox, true);
        assert!(subtitle.starts_with("udp://127.0.0.1:53"), "{subtitle}");
        assert!(!subtitle.contains(PRIVATE_DNS_WARNING), "{subtitle}");
    }

    #[test]
    fn test_primary_row_subtitle_appends_private_warning() {
        let flagged = flagged_server("10.0.0.1", Some("proxy"));
        let subtitle = primary_dns_subtitle(&flagged, BackendType::SingBox, true);
        assert!(subtitle.starts_with("udp://10.0.0.1:53"), "{subtitle}");
        assert!(subtitle.contains(PRIVATE_DNS_WARNING), "{subtitle}");

        let direct = flagged_server("10.0.0.1", Some("direct"));
        let subtitle = primary_dns_subtitle(&direct, BackendType::SingBox, true);
        assert!(!subtitle.contains(PRIVATE_DNS_WARNING), "{subtitle}");
    }

    #[test]
    fn test_primary_row_missing_note_states_skipped_derived_rules() {
        assert_eq!(
            primary_row_missing_note(AUTO_SPLIT_REMOTE_TAG, false),
            "Not configured - routing-derived DNS rules for proxied domains are skipped"
        );
        assert_eq!(
            primary_row_missing_note(AUTO_SPLIT_DOMESTIC_TAG, false),
            "Not configured - routing-derived DNS rules for direct traffic are skipped"
        );
    }

    #[test]
    fn test_primary_row_missing_note_custom_rules_shows_plain_not_configured() {
        assert_eq!(
            primary_row_missing_note(AUTO_SPLIT_REMOTE_TAG, true),
            "Not configured - set up in Advanced"
        );
        assert_eq!(
            primary_row_missing_note(AUTO_SPLIT_DOMESTIC_TAG, true),
            "Not configured - set up in Advanced"
        );
    }

    #[test]
    fn collect_enabled_node_addresses_triggers_xray_tun_bootstrap_rows() {
        use v2ray_rs_core::models::{ProxyNode, SubscriptionNode, TransportSettings, VlessConfig};

        let make_vless = |addr: &str| {
            ProxyNode::Vless(VlessConfig {
                address: addr.into(),
                port: 443,
                uuid: "11111111-1111-1111-1111-111111111111".into(),
                encryption: Some("none".into()),
                flow: None,
                transport: TransportSettings::Tcp,
                tls: None,
                remark: None,
            })
        };

        let mut sub_enabled = Subscription::new_from_url("Enabled", "https://example.com/1");
        sub_enabled.nodes = vec![SubscriptionNode::new(make_vless("edge.example.com")), {
            let mut disabled_node = SubscriptionNode::new(make_vless("disabled.example.com"));
            disabled_node.enabled = false;
            disabled_node
        }];

        let mut sub_disabled = Subscription::new_from_url("Disabled", "https://example.com/2");
        sub_disabled.enabled = false;
        sub_disabled.nodes = vec![SubscriptionNode::new(make_vless("sub-off.example.com"))];

        let mut manual_disabled = ManualNode::new(make_vless("manual-off.example.com"));
        manual_disabled.enabled = false;
        let manual_enabled = ManualNode::new(make_vless("manual.example.com"));

        let addrs = collect_enabled_node_addresses(
            &[sub_enabled, sub_disabled],
            &[manual_enabled, manual_disabled],
        );
        assert_eq!(addrs, vec!["edge.example.com", "manual.example.com"]);

        let mut settings = AppSettings::default();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "remote".into(),
            protocol: DnsProtocol::Doh,
            address: "1.1.1.1".into(),
            port: None,
            detour: Some("proxy".into()),
        }];

        let addr_refs: Vec<&str> = addrs.iter().map(String::as_str).collect();
        let summary = effective_dns(BackendType::Xray, &settings, &[], &addr_refs);
        let bootstrap_scopes: Vec<String> = summary
            .entries()
            .iter()
            .filter(|e| e.source() == DnsSource::Bootstrap)
            .map(|e| e.scope().to_string())
            .collect();
        assert!(
            bootstrap_scopes
                .iter()
                .any(|s| s.contains("edge.example.com")),
            "expected edge.example.com in bootstrap scopes: {bootstrap_scopes:?}"
        );
        assert!(
            bootstrap_scopes
                .iter()
                .any(|s| s.contains("manual.example.com")),
            "expected manual.example.com in bootstrap scopes: {bootstrap_scopes:?}"
        );
    }
}
