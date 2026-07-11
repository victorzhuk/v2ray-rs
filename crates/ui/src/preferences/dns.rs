use adw::prelude::*;
use ipnet::{Ipv4Net, Ipv6Net};
use relm4::adw;
use relm4::gtk;
use std::cell::RefCell;
use std::net::IpAddr;
use std::rc::Rc;
use std::str::FromStr;

use v2ray_rs_core::models::{
    AppSettings, BackendType, DnsProtocol, DnsRule, DnsRuleMatch, DnsServerConfig, DnsStrategy,
    HostOverride, builtin_dns_presets,
};

use super::{SettingsCallback, SettingsObservers, emit, subscribe_settings};

pub(super) fn build_dns_page(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    settings_observers: &SettingsObservers,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("DNS")
        .icon_name("network-transmit-symbolic")
        .build();

    let s = state.borrow();

    let primary_group = adw::PreferencesGroup::builder()
        .title("Primary DNS")
        .build();

    let enable_row = adw::SwitchRow::builder()
        .title("Enable DNS")
        .active(s.dns.enabled)
        .build();
    primary_group.add(&enable_row);

    let strategy_row = adw::ComboRow::builder()
        .title("IP Strategy")
        .model(&gtk::StringList::new(&[
            "Prefer IPv4",
            "Prefer IPv6",
            "IPv4 Only",
            "IPv6 Only",
        ]))
        .selected(strategy_to_index(s.dns.strategy))
        .build();
    primary_group.add(&strategy_row);

    let remote_server_row = adw::ActionRow::builder()
        .title("Remote")
        .subtitle("Not configured")
        .build();
    let remote_edit_btn = gtk::Button::builder()
        .icon_name("document-edit-symbolic")
        .valign(gtk::Align::Center)
        .has_frame(false)
        .css_classes(["flat"])
        .tooltip_text("Edit Remote Server")
        .build();
    remote_server_row.add_suffix(&remote_edit_btn);
    primary_group.add(&remote_server_row);

    let domestic_server_row = adw::ActionRow::builder()
        .title("Domestic")
        .subtitle("Not configured")
        .build();
    let domestic_edit_btn = gtk::Button::builder()
        .icon_name("document-edit-symbolic")
        .valign(gtk::Align::Center)
        .has_frame(false)
        .css_classes(["flat"])
        .tooltip_text("Edit Domestic Server")
        .build();
    domestic_server_row.add_suffix(&domestic_edit_btn);
    primary_group.add(&domestic_server_row);

    page.add(&primary_group);

    drop(s);

    {
        let st = state.clone();
        let cb = cb.clone();
        enable_row.connect_active_notify(move |row| {
            st.borrow_mut().dns.enabled = row.is_active();
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        strategy_row.connect_selected_notify(move |row| {
            st.borrow_mut().dns.strategy = index_to_strategy(row.selected());
            emit(&st, &cb);
        });
    }
    let advanced_expander = adw::ExpanderRow::builder()
        .title("Advanced")
        .subtitle("Full configuration")
        .build();

    let servers_group = adw::PreferencesGroup::builder().title("Servers").build();
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
        let last_backend = Rc::new(RefCell::new(ctx.state.borrow().backend.backend_type));
        subscribe_settings(settings_observers, move |settings| {
            let backend = settings.backend.backend_type;
            sync_dns_ui_observer(settings);
            if backend != *last_backend.borrow() {
                *last_backend.borrow_mut() = backend;
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

#[derive(Clone)]
struct DnsRenderCtx {
    state: Rc<RefCell<AppSettings>>,
    cb: SettingsCallback,
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

fn backend_display_name(b: BackendType) -> &'static str {
    match b {
        BackendType::V2ray => "v2ray",
        BackendType::Xray => "xray",
        BackendType::SingBox => "sing-box",
    }
}

fn validate_dns_settings_for_backend(settings: &AppSettings) -> Result<(), String> {
    settings.dns.validate().map_err(|err| err.to_string())
}

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

fn upsert_dns_server(
    settings: &mut AppSettings,
    original_tag: Option<&str>,
    server: DnsServerConfig,
) -> Result<(), String> {
    if let Some(original_tag) = original_tag {
        let Some(idx) = settings
            .dns
            .servers
            .iter()
            .position(|existing| existing.tag == original_tag)
        else {
            return Err(format!("DNS server '{original_tag}' no longer exists"));
        };

        if original_tag != server.tag {
            for rule in &mut settings.dns.rules {
                if rule.server_tag == original_tag {
                    rule.server_tag = server.tag.clone();
                }
            }
        }

        settings.dns.servers[idx] = server;
    } else {
        settings.dns.servers.push(server);
    }

    Ok(())
}

fn upsert_dns_rule(
    settings: &mut AppSettings,
    original_rule: Option<&DnsRule>,
    rule: DnsRule,
) -> Result<(), String> {
    if let Some(original_rule) = original_rule {
        let Some(idx) = settings
            .dns
            .rules
            .iter()
            .position(|existing| existing == original_rule)
        else {
            return Err("DNS rule no longer exists".to_string());
        };
        settings.dns.rules[idx] = rule;
    } else {
        settings.dns.rules.push(rule);
    }

    Ok(())
}

fn upsert_dns_host(
    settings: &mut AppSettings,
    original_domain: Option<&str>,
    host: HostOverride,
) -> Result<(), String> {
    if let Some(original_domain) = original_domain {
        let Some(idx) = settings
            .dns
            .hosts
            .iter()
            .position(|existing| existing.domain == original_domain)
        else {
            return Err(format!("DNS host '{original_domain}' no longer exists"));
        };
        settings.dns.hosts[idx] = host;
    } else {
        settings.dns.hosts.push(host);
    }

    Ok(())
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

fn dns_server_from_inputs(
    tag_entry: &adw::EntryRow,
    protocol_combo: &adw::ComboRow,
    address_entry: &adw::EntryRow,
    port_spin: &adw::SpinRow,
    is_singbox: bool,
    detour_combo: &adw::ComboRow,
) -> Result<DnsServerConfig, String> {
    let tag = tag_entry.text().trim().to_string();
    if tag.is_empty() {
        return Err("Tag is required".to_string());
    }

    let address = address_entry.text().trim().to_string();
    if address.is_empty() {
        return Err("Address is required".to_string());
    }

    let protocol = index_to_protocol(protocol_combo.selected());
    let port_value = port_spin.value() as u16;
    let port = if port_value == protocol.default_port() {
        None
    } else {
        Some(port_value)
    };

    let detour = if is_singbox {
        Some(if detour_combo.selected() == 1 {
            "direct".to_string()
        } else {
            "proxy".to_string()
        })
    } else {
        None
    };

    Ok(DnsServerConfig {
        tag,
        protocol,
        address,
        port,
        detour,
    })
}

fn dns_rule_from_inputs(
    match_combo: &adw::ComboRow,
    value_entry: &adw::EntryRow,
    server_combo: &adw::ComboRow,
    servers: &[String],
) -> Result<DnsRule, String> {
    let value = value_entry.text().trim().to_string();
    if value.is_empty() {
        return Err("Value is required".to_string());
    }

    let match_condition = match match_combo.selected() {
        1 => DnsRuleMatch::DomainSuffix { suffix: value },
        _ => DnsRuleMatch::GeoSite { category: value },
    };

    let server_idx = server_combo.selected() as usize;
    let Some(server_tag) = servers.get(server_idx) else {
        return Err("Server tag is required".to_string());
    };

    Ok(DnsRule {
        match_condition,
        server_tag: server_tag.clone(),
    })
}

fn dns_host_from_inputs(
    domain_entry: &adw::EntryRow,
    ip_entry: &adw::EntryRow,
) -> Result<HostOverride, String> {
    let domain = domain_entry.text().trim().to_string();
    if domain.is_empty() {
        return Err("Domain is required".to_string());
    }

    let ip = ip_entry.text().trim().to_string();
    if ip.is_empty() {
        return Err("IP address is required".to_string());
    }

    Ok(HostOverride { domain, ip })
}

fn render_dns_servers(ctx: &DnsRenderCtx) {
    for row in ctx.added_servers.borrow().iter() {
        ctx.servers_group.remove(row);
    }
    ctx.added_servers.borrow_mut().clear();

    let servers = ctx.state.borrow().dns.servers.clone();
    let backend = ctx.state.borrow().backend.backend_type;

    let mut added = ctx.added_servers.borrow_mut();
    for server in &servers {
        let protocol_str = format!("{:?}", server.protocol).to_lowercase();

        let mut subtitle = format!(
            "{}://{}:{}",
            protocol_str,
            server.address,
            server
                .port
                .unwrap_or_else(|| server.protocol.default_port())
        );

        if let Some(fallback) = server.protocol.fallback_protocol_for_backend(backend) {
            subtitle.push_str(&format!(
                "\nDowngraded to {} on {}",
                protocol_display_name(fallback),
                backend_display_name(backend)
            ));
        }

        let row = adw::ActionRow::builder()
            .title(&server.tag)
            .subtitle(&subtitle)
            .build();

        let edit_btn = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .valign(gtk::Align::Center)
            .has_frame(false)
            .css_classes(["flat"])
            .build();
        let ctx_clone = ctx.clone();
        let server_clone = server.clone();
        edit_btn.connect_clicked(move |_| {
            show_dns_server_dialog(Some(server_clone.clone()), &ctx_clone);
        });
        row.add_suffix(&edit_btn);

        let remove_btn = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .valign(gtk::Align::Center)
            .has_frame(false)
            .css_classes(["flat", "destructive-action"])
            .build();
        let ctx_clone = ctx.clone();
        let tag = server.tag.clone();
        remove_btn.connect_clicked(move |_| {
            show_dns_remove_server_dialog(tag.clone(), &ctx_clone);
        });
        row.add_suffix(&remove_btn);

        ctx.servers_group.add(&row);
        added.push(row);
    }
}

fn render_dns_rules(ctx: &DnsRenderCtx) {
    for row in ctx.added_rules.borrow().iter() {
        ctx.rules_group.remove(row);
    }
    ctx.added_rules.borrow_mut().clear();

    let use_custom = ctx.state.borrow().dns.use_custom_rules;

    if !use_custom {
        let label = adw::ActionRow::builder()
            .title("Rules auto-derived from routing")
            .subtitle("Enable custom rules to manually configure DNS rules")
            .sensitive(false)
            .build();
        ctx.rules_group.add(&label);
        ctx.added_rules.borrow_mut().push(label);
        return;
    }

    let rules = ctx.state.borrow().dns.rules.clone();

    let mut added = ctx.added_rules.borrow_mut();
    for rule in &rules {
        let (match_type, value) = match &rule.match_condition {
            DnsRuleMatch::GeoSite { category } => ("GeoSite", category),
            DnsRuleMatch::DomainSuffix { suffix } => ("Domain Suffix", suffix),
        };

        let row = adw::ActionRow::builder()
            .title(format!("{match_type}: {value}"))
            .subtitle(format!("Server: {}", rule.server_tag))
            .build();

        let edit_btn = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .valign(gtk::Align::Center)
            .has_frame(false)
            .css_classes(["flat"])
            .build();
        let ctx_clone = ctx.clone();
        let rule_clone = rule.clone();
        edit_btn.connect_clicked(move |_| {
            show_dns_rule_dialog(Some(rule_clone.clone()), &ctx_clone);
        });
        row.add_suffix(&edit_btn);

        let remove_btn = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .valign(gtk::Align::Center)
            .has_frame(false)
            .css_classes(["flat", "destructive-action"])
            .build();
        let ctx_clone = ctx.clone();
        let rule_clone = rule.clone();
        remove_btn.connect_clicked(move |_| {
            if apply_dns_mutation(&ctx_clone, |settings| {
                settings.dns.rules.retain(|r| r != &rule_clone);
                Ok(())
            })
            .is_ok()
            {
                render_dns_rules(&ctx_clone);
            }
        });
        row.add_suffix(&remove_btn);

        ctx.rules_group.add(&row);
        added.push(row);
    }
}

fn render_primary_dns_servers(ctx: &DnsRenderCtx) {
    let servers = ctx.state.borrow().dns.servers.clone();
    let backend = ctx.state.borrow().backend.backend_type;

    let remote_server = servers.iter().find(|s| s.tag == "remote");
    let domestic_server = servers.iter().find(|s| s.tag == "domestic");

    if let Some(server) = remote_server {
        let protocol_str = format!("{:?}", server.protocol).to_lowercase();
        let mut subtitle = format!(
            "{}://{}:{}",
            protocol_str,
            server.address,
            server
                .port
                .unwrap_or_else(|| server.protocol.default_port())
        );
        if let Some(fallback) = server.protocol.fallback_protocol_for_backend(backend) {
            subtitle.push_str(&format!(
                "\nDowngraded to {} on {}",
                protocol_display_name(fallback),
                backend_display_name(backend)
            ));
        }
        ctx.remote_row.set_subtitle(&subtitle);
        ctx.remote_edit_btn.set_sensitive(true);
    } else {
        ctx.remote_row
            .set_subtitle("Not configured - set up in Advanced");
        ctx.remote_edit_btn.set_sensitive(false);
    }

    if let Some(server) = domestic_server {
        let protocol_str = format!("{:?}", server.protocol).to_lowercase();
        let mut subtitle = format!(
            "{}://{}:{}",
            protocol_str,
            server.address,
            server
                .port
                .unwrap_or_else(|| server.protocol.default_port())
        );
        if let Some(fallback) = server.protocol.fallback_protocol_for_backend(backend) {
            subtitle.push_str(&format!(
                "\nDowngraded to {} on {}",
                protocol_display_name(fallback),
                backend_display_name(backend)
            ));
        }
        ctx.domestic_row.set_subtitle(&subtitle);
        ctx.domestic_edit_btn.set_sensitive(true);
    } else {
        ctx.domestic_row
            .set_subtitle("Not configured - set up in Advanced");
        ctx.domestic_edit_btn.set_sensitive(false);
    }
}

fn render_dns_hosts(ctx: &DnsRenderCtx) {
    for row in ctx.added_hosts.borrow().iter() {
        ctx.hosts_group.remove(row);
    }
    ctx.added_hosts.borrow_mut().clear();

    let hosts = ctx.state.borrow().dns.hosts.clone();

    let mut added = ctx.added_hosts.borrow_mut();
    for host in &hosts {
        let row = adw::ActionRow::builder()
            .title(&host.domain)
            .subtitle(&host.ip)
            .build();

        let remove_btn = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .valign(gtk::Align::Center)
            .has_frame(false)
            .css_classes(["flat", "destructive-action"])
            .build();
        let ctx_clone = ctx.clone();
        let domain = host.domain.clone();
        remove_btn.connect_clicked(move |_| {
            if apply_dns_mutation(&ctx_clone, |settings| {
                settings.dns.hosts.retain(|h| h.domain != domain);
                Ok(())
            })
            .is_ok()
            {
                render_dns_hosts(&ctx_clone);
            }
        });
        row.add_suffix(&remove_btn);

        ctx.hosts_group.add(&row);
        added.push(row);
    }
}

fn show_dns_remove_server_dialog(tag: String, ctx: &DnsRenderCtx) {
    let dialog = adw::AlertDialog::builder()
        .heading("Remove DNS Server")
        .body(format!(
            "Are you sure you want to remove the DNS server \"{}\"? Any DNS rules referencing this server will also be removed.",
            tag
        ))
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("remove", "Remove");
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let ctx = ctx.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "remove" {
            return;
        }

        if apply_dns_mutation(&ctx, |settings| {
            settings.dns.servers.retain(|srv| srv.tag != tag);
            settings.dns.rules.retain(|r| r.server_tag != tag);
            Ok(())
        })
        .is_ok()
        {
            render_dns_servers(&ctx);
            render_dns_rules(&ctx);
            render_primary_dns_servers(&ctx);
        }
    });

    dialog.present(crate::active_window().as_ref());
}

fn show_dns_server_dialog(existing: Option<DnsServerConfig>, ctx: &DnsRenderCtx) {
    let is_edit = existing.is_some();

    let (init_tag, init_protocol, init_address, init_port, init_detour) = match &existing {
        Some(srv) => (
            srv.tag.clone(),
            protocol_to_index(srv.protocol),
            srv.address.clone(),
            srv.port.unwrap_or_default(),
            srv.detour.clone().unwrap_or_default(),
        ),
        None => (String::new(), 0, String::new(), 0u16, String::new()),
    };

    let is_singbox = ctx.state.borrow().backend.backend_type == BackendType::SingBox;
    let backend = ctx.state.borrow().backend.backend_type;

    let dialog = adw::AlertDialog::builder()
        .heading(if is_edit {
            "Edit DNS Server"
        } else {
            "Add DNS Server"
        })
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", if is_edit { "Save" } else { "Add" });
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let group = adw::PreferencesGroup::new();

    let tag_entry = adw::EntryRow::builder()
        .title("Tag")
        .text(&init_tag)
        .build();
    group.add(&tag_entry);

    let protocol_combo = adw::ComboRow::builder()
        .title("Protocol")
        .model(&gtk::StringList::new(&[
            "UDP", "TCP", "DoH", "DoT", "DoQ", "H3",
        ]))
        .selected(init_protocol)
        .build();
    group.add(&protocol_combo);

    let address_entry = adw::EntryRow::builder()
        .title("Address")
        .text(&init_address)
        .build();
    group.add(&address_entry);

    let port_spin = adw::SpinRow::builder()
        .title("Port")
        .adjustment(&gtk::Adjustment::new(
            init_port as f64,
            1.0,
            65535.0,
            1.0,
            0.0,
            0.0,
        ))
        .build();
    group.add(&port_spin);

    let detour_combo = adw::ComboRow::builder()
        .title("Detour")
        .visible(is_singbox)
        .model(&gtk::StringList::new(&["proxy", "direct"]))
        .selected(if init_detour == "direct" { 1 } else { 0 })
        .build();
    group.add(&detour_combo);

    let error_label = gtk::Label::builder()
        .label("")
        .css_classes(["error-label"])
        .wrap(true)
        .xalign(0.0)
        .halign(gtk::Align::Start)
        .visible(false)
        .build();

    let warning_label = gtk::Label::builder()
        .label("")
        .wrap(true)
        .xalign(0.0)
        .halign(gtk::Align::Start)
        .visible(false)
        .build();

    content.append(&group);
    content.append(&error_label);
    content.append(&warning_label);
    dialog.set_extra_child(Some(&content));

    let update_warning: Rc<dyn Fn()> = Rc::new({
        let warning_label = warning_label.clone();
        let protocol_combo = protocol_combo.clone();
        move || {
            let protocol = index_to_protocol(protocol_combo.selected());
            if let Some(fallback) = protocol.fallback_protocol_for_backend(backend) {
                warning_label.set_text(&format!(
                    "Effective protocol on {} is {}",
                    backend_display_name(backend),
                    protocol_display_name(fallback)
                ));
                warning_label.set_visible(true);
            } else {
                warning_label.set_text("");
                warning_label.set_visible(false);
            }
        }
    });
    update_warning();
    {
        let update_warning = update_warning.clone();
        protocol_combo.connect_selected_notify(move |_| update_warning());
    }

    let validate: Rc<dyn Fn() -> Result<DnsServerConfig, String>> = Rc::new({
        let state = ctx.state.clone();
        let existing = existing.clone();
        let tag_entry = tag_entry.clone();
        let protocol_combo = protocol_combo.clone();
        let address_entry = address_entry.clone();
        let port_spin = port_spin.clone();
        let detour_combo = detour_combo.clone();
        move || {
            let server = dns_server_from_inputs(
                &tag_entry,
                &protocol_combo,
                &address_entry,
                &port_spin,
                is_singbox,
                &detour_combo,
            )?;

            let mut next = state.borrow().clone();
            upsert_dns_server(
                &mut next,
                existing.as_ref().map(|server| server.tag.as_str()),
                server.clone(),
            )?;
            validate_dns_settings_for_backend(&next)?;

            Ok(server)
        }
    });

    let update_validation: Rc<dyn Fn()> = Rc::new({
        let dialog = dialog.clone();
        let error_label = error_label.clone();
        let validate = validate.clone();
        move || match validate() {
            Ok(_) => {
                set_validation_message(&error_label, None);
                dialog.set_response_enabled("save", true);
            }
            Err(err) => {
                set_validation_message(&error_label, Some(&err));
                dialog.set_response_enabled("save", false);
            }
        }
    });

    update_validation();

    {
        let update_validation = update_validation.clone();
        tag_entry.connect_changed(move |_| update_validation());
    }
    {
        let update_validation = update_validation.clone();
        address_entry.connect_changed(move |_| update_validation());
    }
    {
        let update_validation = update_validation.clone();
        protocol_combo.connect_selected_notify(move |_| update_validation());
    }
    {
        let update_validation = update_validation.clone();
        detour_combo.connect_selected_notify(move |_| update_validation());
    }

    let ctx = ctx.clone();
    let existing_server = existing.clone();
    let error_label_clone = error_label.clone();
    dialog.connect_response(Some("save"), move |_, _| {
        let server = match validate() {
            Ok(server) => server,
            Err(err) => {
                set_validation_message(&error_label_clone, Some(&err));
                return;
            }
        };

        if apply_dns_mutation(&ctx, |settings| {
            upsert_dns_server(
                settings,
                existing_server.as_ref().map(|server| server.tag.as_str()),
                server,
            )
        })
        .is_ok()
        {
            render_dns_servers(&ctx);
            render_dns_rules(&ctx);
            render_primary_dns_servers(&ctx);
        }
    });

    dialog.present(crate::active_window().as_ref());
}

fn show_dns_rule_dialog(existing: Option<DnsRule>, ctx: &DnsRenderCtx) {
    let is_edit = existing.is_some();

    let (init_type, init_value, init_server_tag) = match &existing {
        Some(rule) => {
            let (type_idx, val) = match &rule.match_condition {
                DnsRuleMatch::GeoSite { category } => (0u32, category.clone()),
                DnsRuleMatch::DomainSuffix { suffix } => (1, suffix.clone()),
            };
            (type_idx, val, rule.server_tag.clone())
        }
        None => (0, String::new(), String::new()),
    };

    let servers: Vec<String> = ctx
        .state
        .borrow()
        .dns
        .servers
        .iter()
        .map(|s| s.tag.clone())
        .collect();

    if servers.is_empty() {
        let dialog = adw::AlertDialog::builder()
            .heading("No DNS Servers")
            .body("Please add at least one DNS server before creating DNS rules.")
            .build();
        dialog.add_response("close", "Close");
        dialog.set_default_response(Some("close"));
        dialog.present(crate::active_window().as_ref());
        return;
    }

    let server_tags: Vec<&str> = servers.iter().map(|s| s.as_str()).collect();
    let init_server_idx = if existing.is_some() && !init_server_tag.is_empty() {
        servers
            .iter()
            .position(|s| s == &init_server_tag)
            .unwrap_or(0) as u32
    } else {
        0
    };

    let dialog = adw::AlertDialog::builder()
        .heading(if is_edit {
            "Edit DNS Rule"
        } else {
            "Add DNS Rule"
        })
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", if is_edit { "Save" } else { "Add" });
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let group = adw::PreferencesGroup::new();

    let match_combo = adw::ComboRow::builder()
        .title("Match Type")
        .model(&gtk::StringList::new(&[
            "GeoSite Category",
            "Domain Suffix",
        ]))
        .selected(init_type)
        .build();
    group.add(&match_combo);

    let value_entry = adw::EntryRow::builder()
        .title("Value")
        .text(&init_value)
        .build();
    group.add(&value_entry);

    let server_combo = adw::ComboRow::builder()
        .title("Server Tag")
        .model(&gtk::StringList::new(&server_tags))
        .selected(init_server_idx)
        .build();
    group.add(&server_combo);

    let error_label = gtk::Label::builder()
        .label("")
        .css_classes(["error-label"])
        .wrap(true)
        .xalign(0.0)
        .halign(gtk::Align::Start)
        .visible(false)
        .build();

    content.append(&group);
    content.append(&error_label);
    dialog.set_extra_child(Some(&content));

    let validate: Rc<dyn Fn() -> Result<DnsRule, String>> = Rc::new({
        let state = ctx.state.clone();
        let existing = existing.clone();
        let match_combo = match_combo.clone();
        let value_entry = value_entry.clone();
        let server_combo = server_combo.clone();
        let servers = servers.clone();
        move || {
            let rule = dns_rule_from_inputs(&match_combo, &value_entry, &server_combo, &servers)?;

            let mut next = state.borrow().clone();
            upsert_dns_rule(&mut next, existing.as_ref(), rule.clone())?;
            validate_dns_settings_for_backend(&next)?;

            Ok(rule)
        }
    });

    let update_validation: Rc<dyn Fn()> = Rc::new({
        let dialog = dialog.clone();
        let error_label = error_label.clone();
        let validate = validate.clone();
        move || match validate() {
            Ok(_) => {
                set_validation_message(&error_label, None);
                dialog.set_response_enabled("save", true);
            }
            Err(err) => {
                set_validation_message(&error_label, Some(&err));
                dialog.set_response_enabled("save", false);
            }
        }
    });

    update_validation();

    {
        let update_validation = update_validation.clone();
        value_entry.connect_changed(move |_| update_validation());
    }
    {
        let update_validation = update_validation.clone();
        match_combo.connect_selected_notify(move |_| update_validation());
    }
    {
        let update_validation = update_validation.clone();
        server_combo.connect_selected_notify(move |_| update_validation());
    }

    let ctx = ctx.clone();
    let existing_rule = existing.clone();
    let error_label_clone = error_label.clone();
    dialog.connect_response(Some("save"), move |_, _| {
        let rule = match validate() {
            Ok(rule) => rule,
            Err(err) => {
                set_validation_message(&error_label_clone, Some(&err));
                return;
            }
        };

        if apply_dns_mutation(&ctx, |settings| {
            upsert_dns_rule(settings, existing_rule.as_ref(), rule)
        })
        .is_ok()
        {
            render_dns_rules(&ctx);
        }
    });

    dialog.present(crate::active_window().as_ref());
}

fn show_dns_host_dialog(existing: Option<HostOverride>, ctx: &DnsRenderCtx) {
    let is_edit = existing.is_some();

    let (init_domain, init_ip) = match &existing {
        Some(host) => (host.domain.clone(), host.ip.clone()),
        None => (String::new(), String::new()),
    };

    let dialog = adw::AlertDialog::builder()
        .heading(if is_edit { "Edit Host" } else { "Add Host" })
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", if is_edit { "Save" } else { "Add" });
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let group = adw::PreferencesGroup::new();

    let domain_entry = adw::EntryRow::builder()
        .title("Domain")
        .text(&init_domain)
        .build();
    group.add(&domain_entry);

    let ip_entry = adw::EntryRow::builder()
        .title("IP Address")
        .text(&init_ip)
        .build();
    group.add(&ip_entry);

    let error_label = gtk::Label::builder()
        .label("")
        .css_classes(["error-label"])
        .wrap(true)
        .xalign(0.0)
        .halign(gtk::Align::Start)
        .visible(false)
        .build();

    content.append(&group);
    content.append(&error_label);
    dialog.set_extra_child(Some(&content));

    let validate: Rc<dyn Fn() -> Result<HostOverride, String>> = Rc::new({
        let state = ctx.state.clone();
        let existing = existing.clone();
        let domain_entry = domain_entry.clone();
        let ip_entry = ip_entry.clone();
        move || {
            let host = dns_host_from_inputs(&domain_entry, &ip_entry)?;

            let mut next = state.borrow().clone();
            upsert_dns_host(
                &mut next,
                existing.as_ref().map(|host| host.domain.as_str()),
                host.clone(),
            )?;
            validate_dns_settings_for_backend(&next)?;

            Ok(host)
        }
    });

    let update_validation: Rc<dyn Fn()> = Rc::new({
        let dialog = dialog.clone();
        let error_label = error_label.clone();
        let validate = validate.clone();
        move || match validate() {
            Ok(_) => {
                set_validation_message(&error_label, None);
                dialog.set_response_enabled("save", true);
            }
            Err(err) => {
                set_validation_message(&error_label, Some(&err));
                dialog.set_response_enabled("save", false);
            }
        }
    });

    update_validation();

    {
        let update_validation = update_validation.clone();
        domain_entry.connect_changed(move |_| update_validation());
    }
    {
        let update_validation = update_validation.clone();
        ip_entry.connect_changed(move |_| update_validation());
    }

    let ctx = ctx.clone();
    let existing_host = existing.clone();
    let error_label_clone = error_label.clone();
    dialog.connect_response(Some("save"), move |_, _| {
        let host = match validate() {
            Ok(host) => host,
            Err(err) => {
                set_validation_message(&error_label_clone, Some(&err));
                return;
            }
        };

        if apply_dns_mutation(&ctx, |settings| {
            upsert_dns_host(
                settings,
                existing_host.as_ref().map(|host| host.domain.as_str()),
                host,
            )
        })
        .is_ok()
        {
            render_dns_hosts(&ctx);
        }
    });

    dialog.present(crate::active_window().as_ref());
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
                    ctx_inner
                        .strategy_row
                        .set_selected(strategy_to_index(ctx_inner.state.borrow().dns.strategy));
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
