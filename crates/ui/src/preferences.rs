use adw::prelude::*;
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use relm4::adw;
use relm4::gtk;
use relm4::gtk::glib;
use std::cell::RefCell;
use std::net::IpAddr;
use std::rc::Rc;
use std::str::FromStr;
use uuid::Uuid;

use v2ray_rs_core::backend::{backend_name, detect_all};
use v2ray_rs_core::geodata::GeodataManager;
use v2ray_rs_core::geodata_index::GeodataIndexManager;
use v2ray_rs_core::models::{
    builtin_dns_presets, builtin_presets, AppSettings, AutoResolveStrategy, BackendConfig,
    BackendType, DnsProtocol, DnsRule, DnsRuleMatch, DnsServerConfig, DnsStrategy, HostOverride,
    Language, Preset, RoutingRule, RoutingRuleSet, RuleAction, RuleMatch, validate_rule_match,
};
use v2ray_rs_core::persistence::{self, AppPaths};

type SettingsCallback = Rc<dyn Fn(AppSettings)>;
type RoutingCallback = Rc<dyn Fn()>;

pub fn show_preferences(
    parent: &adw::ApplicationWindow,
    paths: &AppPaths,
    settings: &AppSettings,
    on_settings_changed: impl Fn(AppSettings) + 'static,
    on_routing_changed: impl Fn() + 'static,
) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");

    let cb: SettingsCallback = Rc::new(on_settings_changed);
    let routing_cb: RoutingCallback = Rc::new(on_routing_changed);
    let settings_state = Rc::new(RefCell::new(settings.clone()));

    let system_page = build_system_page(&settings_state, &cb);
    dialog.add(&system_page);

    let network_page = build_network_page(&settings_state, &cb, paths);
    dialog.add(&network_page);

    let routing_page = build_routing_page(paths, &settings_state, routing_cb);
    dialog.add(&routing_page);

    let dns_page = build_dns_page(&settings_state, &cb);
    dialog.add(&dns_page);

    dialog.present(Some(parent));
    dialog
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

    let interface_group = adw::PreferencesGroup::builder().title("Interface").build();

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
    paths: &AppPaths,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Network")
        .icon_name("network-server-symbolic")
        .build();

    let s = state.borrow();
    let backend_type = s.backend.backend_type;

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
                .title(format!(
                    "{} {}",
                    backend_name(backend.backend_type),
                    version_str
                ))
                .subtitle(backend.binary_path.display().to_string())
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

    let geodata_group = adw::PreferencesGroup::builder().title("GeoData").build();

    let _geodata_manager = GeodataManager::new(paths);
    let index_manager = GeodataIndexManager::new(paths);

    let geodata_status = match index_manager.load_index(backend_type) {
        Ok(Some(index)) => {
            let last_refresh = index
                .last_refresh
                .map(|dt| {
                    let local: chrono::DateTime<chrono::Local> = dt.into();
                    local.format("%Y-%m-%d %H:%M").to_string()
                })
                .unwrap_or_else(|| "Never".to_string());
            format!(
                "Last refresh: {} | GeoIP: {} entries | GeoSite: {} entries",
                last_refresh, index.tag_counts.0, index.tag_counts.1
            )
        }
        Ok(None) => "Not indexed".to_string(),
        Err(_) => "Error loading index".to_string(),
    };

    let geodata_row = adw::ActionRow::builder()
        .title("GeoData Status")
        .subtitle(geodata_status)
        .build();
    geodata_group.add(&geodata_row);

    let geodata_btn_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::End)
        .build();

    let geodata_update_btn = gtk::Button::builder()
        .label("Update Now")
        .css_classes(["suggested-action"])
        .build();
    geodata_btn_box.append(&geodata_update_btn);

    let geodata_spinner = gtk::Spinner::builder()
        .visible(false)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    geodata_btn_box.append(&geodata_spinner);

    let geodata_toolbar_row = adw::ActionRow::builder().activatable(false).build();
    geodata_toolbar_row.add_suffix(&geodata_btn_box);
    geodata_group.add(&geodata_toolbar_row);
    page.add(&geodata_group);

    {
        let btn = geodata_update_btn.clone();
        let spinner = geodata_spinner.clone();
        let status_row = geodata_row.clone();
        let paths = paths.clone();
        let st = state.clone();

        geodata_update_btn.connect_clicked(move |_| {
            btn.set_sensitive(false);
            spinner.set_visible(true);
            spinner.start();

            let backend_type = st.borrow().backend.backend_type;
            let geodata_manager = GeodataManager::new(&paths);
            let index_manager = GeodataIndexManager::new(&paths);

            let btn_clone = btn.clone();
            let spinner_clone = spinner.clone();
            let status_row_clone = status_row.clone();

            glib::MainContext::default().spawn_local(async move {
                let result = tokio::task::spawn_blocking(move || -> Result<String, String> {
                    #[cfg(feature = "geodata-fetch")]
                    {
                        use v2ray_rs_core::geodata::download_geodata;

                        download_geodata(&geodata_manager, backend_type)
                            .map_err(|e| format!("Download failed: {}", e))?;

                        let geoip_path = geodata_manager.geoip_path(backend_type);
                        let geosite_path = geodata_manager.geosite_path(backend_type);

                        index_manager
                            .build_index(backend_type, &geoip_path, &geosite_path)
                            .map_err(|e| format!("Index build failed: {}", e))?;

                        Ok("Geodata updated successfully".to_string())
                    }

                    #[cfg(not(feature = "geodata-fetch"))]
                    {
                        Err("Geodata download feature not enabled".to_string())
                    }
                })
                .await;

                spinner_clone.stop();
                spinner_clone.set_visible(false);
                btn_clone.set_sensitive(true);

                match result {
                    Ok(Ok(_)) => {
                        if let Ok(Some(index)) = index_manager.load_index(backend_type) {
                            let last_refresh = index
                                .last_refresh
                                .map(|dt| {
                                    let local: chrono::DateTime<chrono::Local> = dt.into();
                                    local.format("%Y-%m-%d %H:%M").to_string()
                                })
                                .unwrap_or_else(|| "Never".to_string());
                            status_row_clone.set_subtitle(&format!(
                                "Last refresh: {} | GeoIP: {} entries | GeoSite: {} entries",
                                last_refresh, index.tag_counts.0, index.tag_counts.1
                            ));
                        }
                    }
                    Ok(Err(err)) => {
                        status_row_clone.set_subtitle(&format!("Error: {}", err));
                    }
                    Err(join_err) => {
                        status_row_clone
                            .set_subtitle(&format!("Error: task panicked: {}", join_err));
                    }
                }
            });
        });
    }

    let resolve_group = adw::PreferencesGroup::builder().title("Connection").build();

    let resolve_row = adw::ComboRow::builder()
        .title("Auto-resolve strategy")
        .model(&gtk::StringList::new(&[
            "List order",
            "Lowest latency",
            "Random",
            "Last successful",
        ]))
        .selected(match s.auto_resolve_strategy {
            AutoResolveStrategy::ListOrder => 0,
            AutoResolveStrategy::LowestLatency => 1,
            AutoResolveStrategy::Random => 2,
            AutoResolveStrategy::LastSuccessful | AutoResolveStrategy::GeoAware => 3,
        })
        .build();
    resolve_group.add(&resolve_row);
    page.add(&resolve_group);

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
    {
        let st = state.clone();
        let cb = cb.clone();
        resolve_row.connect_selected_notify(move |row| {
            st.borrow_mut().auto_resolve_strategy = match row.selected() {
                1 => AutoResolveStrategy::LowestLatency,
                2 => AutoResolveStrategy::Random,
                3 => AutoResolveStrategy::LastSuccessful,
                _ => AutoResolveStrategy::ListOrder,
            };
            emit(&st, &cb);
        });
    }

    page
}

fn build_routing_page(
    paths: &AppPaths,
    settings_state: &Rc<RefCell<AppSettings>>,
    routing_changed_cb: RoutingCallback,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Routing")
        .icon_name("network-workgroup-symbolic")
        .build();

    let rule_set = persistence::load_routing_rules(paths).unwrap_or_default();
    let rule_set = Rc::new(RefCell::new(rule_set));
    let paths = Rc::new(paths.clone());

    let backend_type = settings_state.borrow().backend.backend_type;

    let toolbar_group = adw::PreferencesGroup::new();

    let toolbar_row = adw::ActionRow::builder().activatable(false).build();

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

    let ctx = RenderCtx {
        page: page.clone(),
        rule_set: rule_set.clone(),
        paths: paths.clone(),
        added_groups: Rc::new(RefCell::new(Vec::new())),
        routing_changed_cb,
        backend_type,
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
    page: adw::PreferencesPage,
    rule_set: Rc<RefCell<RoutingRuleSet>>,
    paths: Rc<AppPaths>,
    added_groups: Rc<RefCell<Vec<adw::PreferencesGroup>>>,
    routing_changed_cb: RoutingCallback,
    backend_type: BackendType,
}

fn render_routing_rules(ctx: &RenderCtx) {
    for g in ctx.added_groups.borrow().iter() {
        ctx.page.remove(g);
    }
    ctx.added_groups.borrow_mut().clear();

    let rs = ctx.rule_set.borrow();
    let rules = rs.rules();

    if rules.is_empty() {
        return;
    }

    let total = rules.len();
    let mut seen: Vec<Option<String>> = Vec::new();
    for rule in rules.iter() {
        if !seen.contains(&rule.group) {
            seen.push(rule.group.clone());
        }
    }

    let mut added = ctx.added_groups.borrow_mut();
    for group_name in &seen {
        let group_rules: Vec<(usize, &RoutingRule)> = rules
            .iter()
            .enumerate()
            .filter(|(_, r)| &r.group == group_name)
            .collect();

        let title = group_name.as_deref().unwrap_or("Custom Rules");
        let pref_group = adw::PreferencesGroup::builder().title(title).build();

        if let Some(name) = group_name {
            let remove_btn = gtk::Button::builder()
                .label("Remove")
                .css_classes(["destructive-action"])
                .valign(gtk::Align::Center)
                .build();
            let gname = name.clone();
            let ctx = ctx.clone();
            remove_btn.connect_clicked(move |_| {
                let ids: Vec<Uuid> = ctx
                    .rule_set
                    .borrow()
                    .rules()
                    .iter()
                    .filter(|r| r.group.as_deref() == Some(&gname))
                    .map(|r| r.id)
                    .collect();
                {
                    let mut rs = ctx.rule_set.borrow_mut();
                    for id in &ids {
                        rs.remove(id);
                    }
                    if let Err(e) = persistence::save_routing_rules(&ctx.paths, &rs) {
                        log::error!("save routing rules: {e}");
                    }
                }
                (ctx.routing_changed_cb)();
                render_routing_rules(&ctx);
            });
            pref_group.set_header_suffix(Some(&remove_btn));
        }

        for (idx, rule) in group_rules {
            let row = build_routing_rule_row(rule, idx, total, ctx);
            pref_group.add(&row);
        }

        ctx.page.add(&pref_group);
        added.push(pref_group);
    }
}

fn build_routing_rule_row(
    rule: &RoutingRule,
    idx: usize,
    total: usize,
    ctx: &RenderCtx,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(format_match(&rule.match_condition))
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
            if let Err(e) = persistence::save_routing_rules(&ctx.paths, &rs) {
                log::error!("save routing rules: {e}");
            }
            (ctx.routing_changed_cb)();
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
            if let Err(e) = persistence::save_routing_rules(&ctx.paths, &ctx.rule_set.borrow()) {
                log::error!("save routing rules: {e}");
            }
            (ctx.routing_changed_cb)();
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
            if let Err(e) = persistence::save_routing_rules(&ctx.paths, &ctx.rule_set.borrow()) {
                log::error!("save routing rules: {e}");
            }
            (ctx.routing_changed_cb)();
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
            if let Err(e) = persistence::save_routing_rules(&ctx.paths, &ctx.rule_set.borrow()) {
                log::error!("save routing rules: {e}");
            }
            (ctx.routing_changed_cb)();
            render_routing_rules(&ctx);
        });
    }
    popover_box.append(&delete_btn);

    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));
    row.add_suffix(&menu_btn);

    // Drag-and-drop support for reordering
    let drag_source = gtk::DragSource::new();
    drag_source.set_actions(gtk::gdk::DragAction::MOVE);

    let drag_idx = idx as u32;
    drag_source.connect_prepare(move |_, _, _| {
        let value = gtk::glib::Value::from(drag_idx);
        let content_provider = gtk::gdk::ContentProvider::for_value(&value);
        Some(content_provider)
    });

    drag_source.connect_drag_begin(|_, _| {});

    row.add_controller(drag_source);

    let drop_target = gtk::DropTarget::new(gtk::glib::Type::U32, gtk::gdk::DragAction::MOVE);

    let ctx_drop = ctx.clone();
    let drop_idx_target = idx;
    drop_target.connect_drop(
        move |target: &gtk::DropTarget, value: &gtk::glib::Value, _, _| {
            let drop_idx = value.get::<u32>().unwrap() as usize;

            if drop_idx == drop_idx_target {
                return false;
            }

            ctx_drop
                .rule_set
                .borrow_mut()
                .move_rule(drop_idx, drop_idx_target);

            if let Err(e) =
                persistence::save_routing_rules(&ctx_drop.paths, &ctx_drop.rule_set.borrow())
            {
                log::error!("save routing rules: {e}");
            }
            (ctx_drop.routing_changed_cb)();
            render_routing_rules(&ctx_drop);

            target.widget().and_then(|w| {
                w.remove_css_class("drop-target");
                Some(())
            });
            true
        },
    );

    drop_target.connect_enter(|target, _, _| {
        target.widget().and_then(|w| {
            w.add_css_class("drop-target");
            Some(())
        });
        gtk::gdk::DragAction::MOVE
    });

    drop_target.connect_leave(|target: &gtk::DropTarget| {
        if let Some(w) = target.widget() {
            w.remove_css_class("drop-target");
        }
    });

    row.add_controller(drop_target);

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

    let type_combo_clone = type_combo.clone();
    let value_entry_clone = value_entry.clone();
    let paths = ctx.paths.clone();
    let backend_type = ctx.backend_type;

    let show_suggestions = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Show suggestions")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();

    {
        let type_combo = type_combo_clone.clone();
        let value_entry = value_entry_clone.clone();
        let paths = paths.clone();
        let backend_type = backend_type;

        show_suggestions.connect_clicked(move |_| {
            let rule_type = type_combo.selected();

            let tags = match rule_type {
                0 => {
                    let manager = GeodataIndexManager::new(&paths);
                    manager
                        .load_index(backend_type)
                        .ok()
                        .flatten()
                        .map(|idx| idx.geoip_tags)
                        .unwrap_or_default()
                }
                1 => {
                    let manager = GeodataIndexManager::new(&paths);
                    manager
                        .load_index(backend_type)
                        .ok()
                        .flatten()
                        .map(|idx| idx.geosite_tags)
                        .unwrap_or_default()
                }
                _ => vec![],
            };

            if tags.is_empty() || (rule_type != 0 && rule_type != 1) {
                return;
            }

            let suggestion_dialog = adw::AlertDialog::builder()
                .heading(match rule_type {
                    0 => "GeoIP Country Codes",
                    1 => "GeoSite Categories",
                    _ => "Suggestions",
                })
                .build();
            suggestion_dialog.add_response("close", "Close");
            suggestion_dialog.set_close_response("close");

            let search_entry = gtk::Entry::builder().placeholder_text("Search...").build();

            let suggestion_list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::Single)
                .css_classes(["navigation-sidebar"])
                .build();

            let suggestion_dialog = Rc::new(suggestion_dialog);
            let tags = Rc::new(tags);
            let value_entry = Rc::new(value_entry.clone());
            let suggestion_list_rc = Rc::new(suggestion_list);

            let list_box_content = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(12)
                .margin_top(12)
                .margin_bottom(12)
                .margin_start(12)
                .margin_end(12)
                .build();
            list_box_content.append(&search_entry);
            list_box_content.append(&*suggestion_list_rc);

            let scrolled = gtk::ScrolledWindow::builder()
                .min_content_height(300)
                .max_content_height(400)
                .child(&list_box_content)
                .build();

            (*suggestion_dialog).set_extra_child(Some(&scrolled));

            let suggestion_dialog_for_list = suggestion_dialog.clone();
            let update_list = {
                let tags = tags.clone();
                let suggestion_list = suggestion_list_rc.clone();
                move |search_text: String| {
                    while let Some(widget) = suggestion_list.first_child() {
                        suggestion_list.remove(&widget);
                    }

                    let filtered: Vec<String> = tags
                        .iter()
                        .filter(|tag| {
                            search_text.is_empty()
                                || tag.to_lowercase().contains(&search_text.to_lowercase())
                        })
                        .cloned()
                        .collect();

                    if filtered.is_empty() {
                        let row = adw::ActionRow::builder()
                            .title("No matching tags")
                            .sensitive(false)
                            .build();
                        suggestion_list.append(&row);
                    } else {
                        for tag in &filtered[..filtered.len().min(20)] {
                            let row = adw::ActionRow::builder()
                                .title(tag)
                                .activatable(true)
                                .build();
                            let tag_clone = tag.clone();
                            let value_entry = value_entry.clone();
                            let suggestion_dialog = suggestion_dialog_for_list.clone();
                            row.connect_activated(move |_| {
                                value_entry.set_text(&tag_clone);
                                suggestion_dialog.close();
                            });
                            suggestion_list.append(&row);
                        }
                        if filtered.len() > 20 {
                            let row = adw::ActionRow::builder()
                                .title(&format!("... and {} more", filtered.len() - 20))
                                .sensitive(false)
                                .build();
                            suggestion_list.append(&row);
                        }
                    }
                }
            };

            update_list(String::new());

            search_entry.connect_changed(move |entry| {
                update_list(entry.text().to_string());
            });

            (*suggestion_dialog).present(gtk::Window::NONE);
        });
    }

    value_entry.add_suffix(&show_suggestions);

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
            0 => {
                let normalized = value.to_uppercase();
                RuleMatch::GeoIp {
                    country_code: normalized,
                }
            }
            1 => {
                let normalized = value.to_lowercase();
                RuleMatch::GeoSite {
                    category: normalized,
                }
            }
            2 => RuleMatch::Domain { pattern: value },
            3 => match IpNet::from_str(&value) {
                Ok(cidr) => RuleMatch::IpCidr { cidr },
                Err(_) => return,
            },
            _ => return,
        };

        if let Err(e) = validate_rule_match(&match_condition) {
            value_entry.add_css_class("error");
            log::warn!("invalid rule match: {e}");
            return;
        }
        value_entry.remove_css_class("error");

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
            group: None,
        };

        {
            let mut rs = ctx.rule_set.borrow_mut();
            let existing_idx = rs.rules().iter().position(|r| r.id == rule.id);
            if let Some(idx) = existing_idx {
                rs.rules_mut()[idx] = rule;
            } else {
                rs.add(rule);
            }
            if let Err(e) = persistence::save_routing_rules(&ctx.paths, &rs) {
                log::error!("save routing rules: {e}");
            }
        }
        (ctx.routing_changed_cb)();
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

    let builtin_group = adw::PreferencesGroup::builder().title("Built-in").build();
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
            if let Err(e) = persistence::save_routing_rules(&ctx.paths, &ctx.rule_set.borrow()) {
                log::error!("save routing rules: {e}");
            }
            (ctx.routing_changed_cb)();
            render_routing_rules(&ctx);
        });
        row.add_suffix(&apply_btn);
        builtin_group.add(&row);
    }
    content.append(&builtin_group);

    let custom = persistence::load_custom_presets(paths).unwrap_or_default();
    if !custom.is_empty() {
        let custom_group = adw::PreferencesGroup::builder().title("Custom").build();
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
                if let Err(e) = persistence::save_routing_rules(&ctx.paths, &ctx.rule_set.borrow())
                {
                    log::error!("save routing rules: {e}");
                }
                (ctx.routing_changed_cb)();
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
                if let Err(e) = persistence::delete_preset(&pp, &name) {
                    log::error!("delete preset: {e}");
                }
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
        if let Err(e) = persistence::save_preset(&paths, &preset) {
            log::error!("save preset: {e}");
        }
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

fn build_dns_page(state: &Rc<RefCell<AppSettings>>, cb: &SettingsCallback) -> adw::PreferencesPage {
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

    let st = state.clone();
    let cb_clone = cb.clone();
    let add_rule_btn_clone = add_rule_btn.clone();
    custom_rules_switch.connect_active_notify(move |sw| {
        st.borrow_mut().dns.use_custom_rules = sw.is_active();
        add_rule_btn_clone.set_sensitive(sw.is_active());
        emit(&st, &cb_clone);
    });

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
                    if trimmed.is_empty() {
                        st.borrow_mut().dns.client_subnet = None;
                    } else {
                        st.borrow_mut().dns.client_subnet = Some(trimmed.to_string());
                    }
                    error_label.set_visible(false);
                    row.remove_css_class("error");
                }
                Err(msg) => {
                    error_label.set_text(&msg);
                    error_label.set_visible(true);
                    row.add_css_class("error");
                }
            }
            emit(&st, &cb);
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
            st.borrow_mut().dns.fakeip.enabled = enabled;

            if enabled {
                let inet4_text = inet4_row.text().to_string();
                let inet6_text = inet6_row.text().to_string();

                if let Err(msg) = validate_ipv4_cidr(&inet4_text) {
                    inet4_error.set_text(&msg);
                    inet4_error.set_visible(true);
                    inet4_row.add_css_class("error");
                }

                if let Err(msg) = validate_ipv6_cidr(&inet6_text) {
                    inet6_error.set_text(&msg);
                    inet6_error.set_visible(true);
                    inet6_row.add_css_class("error");
                }
            } else {
                inet4_error.set_visible(false);
                inet6_error.set_visible(false);
                inet4_row.remove_css_class("error");
                inet6_row.remove_css_class("error");
            }
            emit(&st, &cb);
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
            st.borrow_mut().dns.fakeip.inet4_range = text.clone();

            let enabled = enable_row.is_active();
            if enabled {
                match validate_ipv4_cidr(&text) {
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
            } else {
                error_label.set_visible(false);
                row.remove_css_class("error");
            }
            emit(&st, &cb);
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
            st.borrow_mut().dns.fakeip.inet6_range = text.clone();

            let enabled = enable_row.is_active();
            if enabled {
                match validate_ipv6_cidr(&text) {
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
            } else {
                error_label.set_visible(false);
                row.remove_css_class("error");
            }
            emit(&st, &cb);
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
        return Ok(()); // Optional field, empty is valid
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

fn render_dns_servers(ctx: &DnsRenderCtx) {
    for row in ctx.added_servers.borrow().iter() {
        ctx.servers_group.remove(row);
    }
    ctx.added_servers.borrow_mut().clear();

    let servers = ctx.state.borrow().dns.servers.clone();

    let mut added = ctx.added_servers.borrow_mut();
    for server in &servers {
        let protocol_str = format!("{:?}", server.protocol).to_lowercase();

        let subtitle = format!(
            "{}://{}:{}",
            protocol_str,
            server.address,
            server
                .port
                .unwrap_or_else(|| server.protocol.default_port())
        );

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
            ctx_clone
                .state
                .borrow_mut()
                .dns
                .rules
                .retain(|r| r != &rule_clone);
            emit(&ctx_clone.state, &ctx_clone.cb);
            render_dns_rules(&ctx_clone);
        });
        row.add_suffix(&remove_btn);

        ctx.rules_group.add(&row);
        added.push(row);
    }
}

fn render_primary_dns_servers(ctx: &DnsRenderCtx) {
    let servers = ctx.state.borrow().dns.servers.clone();

    let remote_server = servers.iter().find(|s| s.tag == "remote");
    let domestic_server = servers.iter().find(|s| s.tag == "domestic");

    if let Some(server) = remote_server {
        let protocol_str = format!("{:?}", server.protocol).to_lowercase();
        let subtitle = format!(
            "{}://{}:{}",
            protocol_str,
            server.address,
            server
                .port
                .unwrap_or_else(|| server.protocol.default_port())
        );
        ctx.remote_row.set_subtitle(&subtitle);

        let ctx_clone = ctx.clone();
        let server_clone = server.clone();
        ctx.remote_edit_btn.connect_clicked(move |_| {
            show_dns_server_dialog(Some(server_clone.clone()), &ctx_clone);
        });
        ctx.remote_edit_btn.set_sensitive(true);
    } else {
        ctx.remote_row
            .set_subtitle("Not configured - set up in Advanced");
        ctx.remote_edit_btn.set_sensitive(false);
    }

    if let Some(server) = domestic_server {
        let protocol_str = format!("{:?}", server.protocol).to_lowercase();
        let subtitle = format!(
            "{}://{}:{}",
            protocol_str,
            server.address,
            server
                .port
                .unwrap_or_else(|| server.protocol.default_port())
        );
        ctx.domestic_row.set_subtitle(&subtitle);

        let ctx_clone = ctx.clone();
        let server_clone = server.clone();
        ctx.domestic_edit_btn.connect_clicked(move |_| {
            show_dns_server_dialog(Some(server_clone.clone()), &ctx_clone);
        });
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
            ctx_clone
                .state
                .borrow_mut()
                .dns
                .hosts
                .retain(|h| h.domain != domain);
            emit(&ctx_clone.state, &ctx_clone.cb);
            render_dns_hosts(&ctx_clone);
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

        {
            let mut s = ctx.state.borrow_mut();
            s.dns.servers.retain(|srv| srv.tag != tag);
            s.dns.rules.retain(|r| r.server_tag != tag);
        }
        emit(&ctx.state, &ctx.cb);
        render_dns_servers(&ctx);
        render_dns_rules(&ctx);
        render_primary_dns_servers(&ctx);
    });

    dialog.present(gtk::Window::NONE);
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
        .model(&gtk::StringList::new(&["proxy-0", "direct"]))
        .selected(if init_detour == "direct" { 1 } else { 0 })
        .build();
    group.add(&detour_combo);

    content.append(&group);
    dialog.set_extra_child(Some(&content));

    let ctx = ctx.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }

        let tag = tag_entry.text().to_string().trim().to_string();
        if tag.is_empty() {
            return;
        }

        let address = address_entry.text().to_string().trim().to_string();
        if address.is_empty() {
            return;
        }

        let protocol = index_to_protocol(protocol_combo.selected());

        let port = {
            let p = port_spin.value() as u16;
            if p == protocol.default_port() {
                None
            } else {
                Some(p)
            }
        };

        let detour = if is_singbox {
            let sel = detour_combo.selected();
            if sel == 1 {
                Some("direct".to_string())
            } else {
                Some("proxy-0".to_string())
            }
        } else {
            None
        };

        let server = DnsServerConfig {
            tag: tag.clone(),
            protocol,
            address,
            port,
            detour,
        };

        {
            let mut s = ctx.state.borrow_mut();
            if is_edit {
                if let Some(idx) = s.dns.servers.iter().position(|srv| srv.tag == tag) {
                    s.dns.servers[idx] = server;
                }
            } else {
                s.dns.servers.push(server);
            }
        }
        emit(&ctx.state, &ctx.cb);
        render_dns_servers(&ctx);
        render_primary_dns_servers(&ctx);
    });

    dialog.present(gtk::Window::NONE);
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
        dialog.present(gtk::Window::NONE);
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

    content.append(&group);
    dialog.set_extra_child(Some(&content));

    let ctx = ctx.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }

        let value = value_entry.text().to_string().trim().to_string();
        if value.is_empty() {
            return;
        }

        let match_condition = match match_combo.selected() {
            1 => DnsRuleMatch::DomainSuffix { suffix: value },
            _ => DnsRuleMatch::GeoSite { category: value },
        };

        let server_idx = server_combo.selected() as usize;
        let server_tag = if server_idx < servers.len() {
            servers[server_idx].clone()
        } else {
            return;
        };

        let rule = DnsRule {
            match_condition,
            server_tag,
        };

        {
            let mut s = ctx.state.borrow_mut();
            if is_edit {
                if let Some(idx) = s.dns.rules.iter().position(|r| {
                    std::mem::discriminant(&r.match_condition)
                        == std::mem::discriminant(&rule.match_condition)
                }) {
                    s.dns.rules[idx] = rule;
                }
            } else {
                s.dns.rules.push(rule);
            }
        }
        emit(&ctx.state, &ctx.cb);
        render_dns_rules(&ctx);
    });

    dialog.present(gtk::Window::NONE);
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

    content.append(&group);
    dialog.set_extra_child(Some(&content));

    let ctx = ctx.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }

        let domain = domain_entry.text().to_string().trim().to_string();
        if domain.is_empty() {
            return;
        }

        let ip = ip_entry.text().to_string().trim().to_string();
        if ip.is_empty() {
            return;
        }

        let host = HostOverride { domain, ip };

        {
            let mut s = ctx.state.borrow_mut();
            if is_edit {
                if let Some(idx) = s.dns.hosts.iter().position(|h| h.domain == host.domain) {
                    s.dns.hosts[idx] = host;
                }
            } else {
                s.dns.hosts.push(host);
            }
        }
        emit(&ctx.state, &ctx.cb);
        render_dns_hosts(&ctx);
    });

    dialog.present(gtk::Window::NONE);
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

                {
                    let mut s = ctx_inner.state.borrow_mut();
                    s.dns.apply_dns_preset(&p_inner);
                }
                emit(&ctx_inner.state, &ctx_inner.cb);
                render_dns_servers(&ctx_inner);
                render_primary_dns_servers(&ctx_inner);
                ctx_inner
                    .strategy_row
                    .set_selected(strategy_to_index(ctx_inner.state.borrow().dns.strategy));
                pd.close();
            });

            confirm_dialog.present(gtk::Window::NONE);
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
    dialog.present(gtk::Window::NONE);
}
