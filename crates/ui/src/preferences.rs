use adw::prelude::*;
use relm4::adw;
use relm4::gtk;
use ipnet::IpNet;
use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;
use uuid::Uuid;

use v2ray_rs_core::backend::{backend_name, detect_all};
use v2ray_rs_core::models::{
    builtin_presets, AppSettings, BackendConfig, Language,
    Preset, RoutingRule, RoutingRuleSet, RuleAction, RuleMatch,
};
use v2ray_rs_core::persistence::{self, AppPaths};

type SettingsCallback = Rc<dyn Fn(AppSettings)>;

pub fn show_preferences(
    parent: &adw::ApplicationWindow,
    paths: &AppPaths,
    settings: &AppSettings,
    on_settings_changed: impl Fn(AppSettings) + 'static,
) {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");

    let cb: SettingsCallback = Rc::new(on_settings_changed);
    let settings_state = Rc::new(RefCell::new(settings.clone()));

    let system_page = build_system_page(&settings_state, &cb);
    dialog.add(&system_page);

    let network_page = build_network_page(&settings_state, &cb);
    dialog.add(&network_page);

    let routing_page = build_routing_page(paths);
    dialog.add(&routing_page);

    dialog.present(Some(parent));
}

fn emit(state: &Rc<RefCell<AppSettings>>, cb: &SettingsCallback) {
    cb(state.borrow().clone());
}

fn build_system_page(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("System")
        .icon_name("preferences-system-symbolic")
        .build();

    let s = state.borrow();

    let interface_group = adw::PreferencesGroup::builder()
        .title("Interface")
        .build();

    let lang_row = adw::ComboRow::builder()
        .title("Language")
        .model(&gtk::StringList::new(&["English", "Russian"]))
        .selected(match s.language {
            Language::English => 0,
            Language::Russian => 1,
        })
        .build();
    interface_group.add(&lang_row);
    page.add(&interface_group);

    let integration_group = adw::PreferencesGroup::builder()
        .title("Integration")
        .build();

    let tray_row = adw::SwitchRow::builder()
        .title("Minimize to tray")
        .active(s.minimize_to_tray)
        .build();
    integration_group.add(&tray_row);

    let notif_row = adw::SwitchRow::builder()
        .title("Enable notifications")
        .active(s.notifications_enabled)
        .build();
    integration_group.add(&notif_row);
    page.add(&integration_group);

    drop(s);

    {
        let st = state.clone();
        let cb = cb.clone();
        lang_row.connect_selected_notify(move |row| {
            st.borrow_mut().language = match row.selected() {
                1 => Language::Russian,
                _ => Language::English,
            };
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        tray_row.connect_active_notify(move |row| {
            st.borrow_mut().minimize_to_tray = row.is_active();
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        notif_row.connect_active_notify(move |row| {
            st.borrow_mut().notifications_enabled = row.is_active();
            emit(&st, &cb);
        });
    }

    page
}

fn build_network_page(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Network")
        .icon_name("network-server-symbolic")
        .build();

    let s = state.borrow();

    let backend_group = adw::PreferencesGroup::builder()
        .title("Backend")
        .description("Select proxy backend")
        .build();

    let detected = detect_all();

    if detected.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No backend found")
            .subtitle("Install v2ray, xray, or sing-box")
            .sensitive(false)
            .build();
        backend_group.add(&row);
    } else {
        let mut first_check: Option<gtk::CheckButton> = None;
        for backend in &detected {
            let version_str = backend
                .version
                .as_ref()
                .map(|v| format!("({})", v))
                .unwrap_or_default();

            let row = adw::ActionRow::builder()
                .title(&format!(
                    "{} {}",
                    backend_name(backend.backend_type),
                    version_str
                ))
                .subtitle(&backend.binary_path.display().to_string())
                .activatable(true)
                .build();

            let check = gtk::CheckButton::builder()
                .active(s.backend.backend_type == backend.backend_type)
                .valign(gtk::Align::Center)
                .build();

            if let Some(ref first) = first_check {
                check.set_group(Some(first));
            } else {
                first_check = Some(check.clone());
            }

            let bt = backend.backend_type;
            let path = backend.binary_path.clone();
            let st = state.clone();
            let cb = cb.clone();
            check.connect_toggled(move |btn| {
                if btn.is_active() {
                    let mut ss = st.borrow_mut();
                    ss.backend = BackendConfig {
                        backend_type: bt,
                        binary_path: Some(path.clone()),
                        config_output_dir: ss.backend.config_output_dir.clone(),
                    };
                    drop(ss);
                    emit(&st, &cb);
                }
            });

            row.add_suffix(&check);
            backend_group.add(&row);
        }
    }
    page.add(&backend_group);

    let ports_group = adw::PreferencesGroup::builder()
        .title("Proxy Ports")
        .build();

    let socks_row = adw::SpinRow::builder()
        .title("SOCKS5 Port")
        .adjustment(&gtk::Adjustment::new(
            s.socks_port as f64,
            1024.0,
            65535.0,
            1.0,
            0.0,
            0.0,
        ))
        .build();
    ports_group.add(&socks_row);

    let http_row = adw::SpinRow::builder()
        .title("HTTP Port")
        .adjustment(&gtk::Adjustment::new(
            s.http_port as f64,
            1024.0,
            65535.0,
            1.0,
            0.0,
            0.0,
        ))
        .build();
    ports_group.add(&http_row);
    page.add(&ports_group);

    let sub_group = adw::PreferencesGroup::builder()
        .title("Subscriptions")
        .build();

    let auto_update_row = adw::SwitchRow::builder()
        .title("Auto-update subscriptions")
        .active(s.auto_update_subscriptions)
        .build();
    sub_group.add(&auto_update_row);

    let interval_row = adw::SpinRow::builder()
        .title("Update interval (hours)")
        .sensitive(s.auto_update_subscriptions)
        .adjustment(&gtk::Adjustment::new(
            (s.subscription_update_interval_secs / 3600) as f64,
            1.0,
            168.0,
            1.0,
            0.0,
            0.0,
        ))
        .build();
    sub_group.add(&interval_row);
    page.add(&sub_group);

    drop(s);

    {
        let st = state.clone();
        let cb = cb.clone();
        socks_row.connect_changed(move |row| {
            st.borrow_mut().socks_port = row.value() as u16;
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        http_row.connect_changed(move |row| {
            st.borrow_mut().http_port = row.value() as u16;
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        let interval = interval_row.clone();
        auto_update_row.connect_active_notify(move |row| {
            st.borrow_mut().auto_update_subscriptions = row.is_active();
            interval.set_sensitive(row.is_active());
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        interval_row.connect_changed(move |row| {
            st.borrow_mut().subscription_update_interval_secs = row.value() as u64 * 3600;
            emit(&st, &cb);
        });
    }

    page
}

fn build_routing_page(paths: &AppPaths) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Routing")
        .icon_name("network-workgroup-symbolic")
        .build();

    let rule_set = persistence::load_routing_rules(paths).unwrap_or_default();
    let rule_set = Rc::new(RefCell::new(rule_set));
    let paths = Rc::new(paths.clone());

    let toolbar_group = adw::PreferencesGroup::new();

    let toolbar_row = adw::ActionRow::builder()
        .activatable(false)
        .build();

    let toolbar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::End)
        .hexpand(true)
        .build();

    let presets_btn = gtk::Button::builder()
        .label("Presets")
        .css_classes(["flat"])
        .build();
    toolbar.append(&presets_btn);

    let add_btn = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add Rule")
        .css_classes(["flat"])
        .build();
    toolbar.append(&add_btn);

    toolbar_row.add_suffix(&toolbar);
    toolbar_group.add(&toolbar_row);
    page.add(&toolbar_group);

    let rules_group = adw::PreferencesGroup::builder()
        .title("Rules")
        .description("Rules are evaluated in order from top to bottom")
        .build();
    page.add(&rules_group);

    let ctx = RenderCtx {
        rules_group: rules_group.clone(),
        rule_set: rule_set.clone(),
        paths: paths.clone(),
        added_rows: Rc::new(RefCell::new(Vec::new())),
    };

    render_routing_rules(&ctx);

    {
        let ctx = ctx.clone();
        add_btn.connect_clicked(move |_| {
            show_routing_rule_dialog(None, &ctx);
        });
    }
    {
        let ctx = ctx.clone();
        let p = paths.clone();
        presets_btn.connect_clicked(move |_| {
            show_routing_presets_dialog(&p, &ctx);
        });
    }

    page
}

#[derive(Clone)]
struct RenderCtx {
    rules_group: adw::PreferencesGroup,
    rule_set: Rc<RefCell<RoutingRuleSet>>,
    paths: Rc<AppPaths>,
    added_rows: Rc<RefCell<Vec<adw::ActionRow>>>,
}

fn render_routing_rules(ctx: &RenderCtx) {
    let group = &ctx.rules_group;

    for row in ctx.added_rows.borrow().iter() {
        group.remove(row);
    }
    ctx.added_rows.borrow_mut().clear();

    let rs = ctx.rule_set.borrow();
    let rules = rs.rules();

    if rules.is_empty() {
        return;
    }

    let total = rules.len();
    let mut rows = ctx.added_rows.borrow_mut();
    for (idx, rule) in rules.iter().enumerate() {
        let row = build_routing_rule_row(rule, idx, total, ctx);
        group.add(&row);
        rows.push(row);
    }
}

fn build_routing_rule_row(
    rule: &RoutingRule,
    idx: usize,
    total: usize,
    ctx: &RenderCtx,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(&format_match(&rule.match_condition))
        .subtitle(format_action(&rule.action))
        .build();

    let switch = gtk::Switch::builder()
        .active(rule.enabled)
        .valign(gtk::Align::Center)
        .build();
    {
        let id = rule.id;
        let ctx = ctx.clone();
        switch.connect_active_notify(move |_| {
            let mut rs = ctx.rule_set.borrow_mut();
            if let Some(r) = rs.rules_mut().iter_mut().find(|r| r.id == id) {
                r.enabled = !r.enabled;
            }
            let _ = persistence::save_routing_rules(&ctx.paths, &rs);
        });
    }
    row.add_suffix(&switch);

    let menu_btn = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .valign(gtk::Align::Center)
        .has_frame(false)
        .css_classes(["flat"])
        .build();

    let popover = gtk::Popover::new();
    let popover_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();

    if idx > 0 {
        let btn = gtk::Button::builder()
            .label("Move Up")
            .has_frame(false)
            .build();
        let ctx = ctx.clone();
        let pop = popover.clone();
        btn.connect_clicked(move |_| {
            pop.popdown();
            ctx.rule_set.borrow_mut().move_rule(idx, idx - 1);
            let _ = persistence::save_routing_rules(&ctx.paths, &ctx.rule_set.borrow());
            render_routing_rules(&ctx);
        });
        popover_box.append(&btn);
    }

    if idx < total - 1 {
        let btn = gtk::Button::builder()
            .label("Move Down")
            .has_frame(false)
            .build();
        let ctx = ctx.clone();
        let pop = popover.clone();
        btn.connect_clicked(move |_| {
            pop.popdown();
            ctx.rule_set.borrow_mut().move_rule(idx, idx + 1);
            let _ = persistence::save_routing_rules(&ctx.paths, &ctx.rule_set.borrow());
            render_routing_rules(&ctx);
        });
        popover_box.append(&btn);
    }

    let edit_btn = gtk::Button::builder()
        .label("Edit")
        .has_frame(false)
        .build();
    {
        let id = rule.id;
        let ctx = ctx.clone();
        let pop = popover.clone();
        edit_btn.connect_clicked(move |_| {
            pop.popdown();
            let rule = ctx
                .rule_set
                .borrow()
                .rules()
                .iter()
                .find(|r| r.id == id)
                .cloned();
            if let Some(r) = rule {
                show_routing_rule_dialog(Some(r), &ctx);
            }
        });
    }
    popover_box.append(&edit_btn);

    let delete_btn = gtk::Button::builder()
        .label("Delete")
        .has_frame(false)
        .css_classes(["destructive-action"])
        .build();
    {
        let id = rule.id;
        let ctx = ctx.clone();
        let pop = popover.clone();
        delete_btn.connect_clicked(move |_| {
            pop.popdown();
            ctx.rule_set.borrow_mut().remove(&id);
            let _ = persistence::save_routing_rules(&ctx.paths, &ctx.rule_set.borrow());
            render_routing_rules(&ctx);
        });
    }
    popover_box.append(&delete_btn);

    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));
    row.add_suffix(&menu_btn);

    row
}

fn show_routing_rule_dialog(existing: Option<RoutingRule>, ctx: &RenderCtx) {
    let is_edit = existing.is_some();

    let dialog = adw::AlertDialog::builder()
        .heading(if is_edit { "Edit Rule" } else { "Add Rule" })
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", if is_edit { "Save" } else { "Add" });
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");

    let (init_type_idx, init_value, init_action_idx, editing_id) = match &existing {
        Some(rule) => {
            let (ti, val) = match &rule.match_condition {
                RuleMatch::GeoIp { country_code } => (0u32, country_code.clone()),
                RuleMatch::GeoSite { category } => (1, category.clone()),
                RuleMatch::Domain { pattern } => (2, pattern.clone()),
                RuleMatch::IpCidr { cidr } => (3, cidr.to_string()),
            };
            let ai = match rule.action {
                RuleAction::Proxy => 0u32,
                RuleAction::Direct => 1,
                RuleAction::Block => 2,
            };
            (ti, val, ai, Some(rule.id))
        }
        None => (0, String::new(), 0, None),
    };

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let type_combo = adw::ComboRow::builder()
        .title("Rule Type")
        .model(&gtk::StringList::new(&[
            "GeoIP Country Code",
            "GeoSite Category",
            "Domain Pattern",
            "IP CIDR",
        ]))
        .selected(init_type_idx)
        .build();

    let value_entry = adw::EntryRow::builder()
        .title("Match Value")
        .text(&init_value)
        .build();

    let action_combo = adw::ComboRow::builder()
        .title("Action")
        .model(&gtk::StringList::new(&["Proxy", "Direct", "Block"]))
        .selected(init_action_idx)
        .build();

    let group = adw::PreferencesGroup::new();
    group.add(&type_combo);
    group.add(&value_entry);
    group.add(&action_combo);
    content.append(&group);

    dialog.set_extra_child(Some(&content));

    let ctx = ctx.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }

        let value = value_entry.text().to_string();
        if value.trim().is_empty() {
            return;
        }
        let value = value.trim().to_string();

        let match_condition = match type_combo.selected() {
            0 => RuleMatch::GeoIp {
                country_code: value,
            },
            1 => RuleMatch::GeoSite { category: value },
            2 => RuleMatch::Domain { pattern: value },
            3 => match IpNet::from_str(&value) {
                Ok(cidr) => RuleMatch::IpCidr { cidr },
                Err(_) => return,
            },
            _ => return,
        };

        let action = match action_combo.selected() {
            0 => RuleAction::Proxy,
            1 => RuleAction::Direct,
            _ => RuleAction::Block,
        };

        let rule = RoutingRule {
            id: editing_id.unwrap_or_else(Uuid::new_v4),
            match_condition,
            action,
            enabled: true,
        };

        {
            let mut rs = ctx.rule_set.borrow_mut();
            let existing_idx = rs.rules().iter().position(|r| r.id == rule.id);
            if let Some(idx) = existing_idx {
                rs.rules_mut()[idx] = rule;
            } else {
                rs.add(rule);
            }
            let _ = persistence::save_routing_rules(&ctx.paths, &rs);
        }
        render_routing_rules(&ctx);
    });

    dialog.present(gtk::Window::NONE);
}

fn show_routing_presets_dialog(paths: &Rc<AppPaths>, ctx: &RenderCtx) {
    let dialog = adw::AlertDialog::builder()
        .heading("Routing Presets")
        .build();
    dialog.add_response("close", "Close");
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();

    let builtin_group = adw::PreferencesGroup::builder()
        .title("Built-in")
        .build();
    for preset in builtin_presets() {
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
        apply_btn.connect_clicked(move |_| {
            ctx.rule_set.borrow_mut().apply_preset(&p);
            let _ = persistence::save_routing_rules(&ctx.paths, &ctx.rule_set.borrow());
            render_routing_rules(&ctx);
        });
        row.add_suffix(&apply_btn);
        builtin_group.add(&row);
    }
    content.append(&builtin_group);

    let custom = persistence::load_custom_presets(paths).unwrap_or_default();
    if !custom.is_empty() {
        let custom_group = adw::PreferencesGroup::builder()
            .title("Custom")
            .build();
        for preset in &custom {
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
            apply_btn.connect_clicked(move |_| {
                ctx.rule_set.borrow_mut().apply_preset(&p);
                let _ = persistence::save_routing_rules(&ctx.paths, &ctx.rule_set.borrow());
                render_routing_rules(&ctx);
            });
            row.add_suffix(&apply_btn);

            let delete_btn = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .valign(gtk::Align::Center)
                .has_frame(false)
                .build();
            let name = preset.name.clone();
            let pp = paths.clone();
            delete_btn.connect_clicked(move |_| {
                let _ = persistence::delete_preset(&pp, &name);
            });
            row.add_suffix(&delete_btn);

            custom_group.add(&row);
        }
        content.append(&custom_group);
    }

    let save_group = adw::PreferencesGroup::new();
    let save_row = adw::ActionRow::builder()
        .title("Save Current Rules as Preset")
        .activatable(true)
        .build();
    save_row.add_prefix(
        &gtk::Image::builder()
            .icon_name("document-save-symbolic")
            .build(),
    );
    {
        let rs = ctx.rule_set.clone();
        let pp = paths.clone();
        save_row.connect_activated(move |_| {
            show_save_preset_dialog(&rs.borrow(), &pp);
        });
    }
    save_group.add(&save_row);
    content.append(&save_group);

    let scrolled = gtk::ScrolledWindow::builder()
        .min_content_height(300)
        .max_content_height(500)
        .child(&content)
        .build();

    dialog.set_extra_child(Some(&scrolled));
    dialog.present(gtk::Window::NONE);
}

fn show_save_preset_dialog(rule_set: &RoutingRuleSet, paths: &AppPaths) {
    let rules: Vec<RoutingRule> = rule_set.rules().to_vec();
    let paths = paths.clone();

    let dialog = adw::AlertDialog::builder()
        .heading("Save as Preset")
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", "Save");
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
    let name_entry = adw::EntryRow::builder().title("Name").build();
    let desc_entry = adw::EntryRow::builder().title("Description").build();
    group.add(&name_entry);
    group.add(&desc_entry);
    content.append(&group);

    dialog.set_extra_child(Some(&content));

    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }
        let name = name_entry.text().to_string();
        if name.trim().is_empty() {
            return;
        }
        let description = desc_entry.text().to_string();
        let preset = Preset::from_rules(name.trim(), description.trim(), &rules);
        let _ = persistence::save_preset(&paths, &preset);
    });

    dialog.present(gtk::Window::NONE);
}

fn format_action(action: &RuleAction) -> &'static str {
    match action {
        RuleAction::Proxy => "Proxy",
        RuleAction::Direct => "Direct",
        RuleAction::Block => "Block",
    }
}

fn format_match(m: &RuleMatch) -> String {
    match m {
        RuleMatch::GeoIp { country_code } => format!("GeoIP: {country_code}"),
        RuleMatch::GeoSite { category } => format!("GeoSite: {category}"),
        RuleMatch::Domain { pattern } => format!("Domain: {pattern}"),
        RuleMatch::IpCidr { cidr } => format!("IP CIDR: {cidr}"),
    }
}
