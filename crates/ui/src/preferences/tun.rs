use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use relm4::{adw, gtk};

use v2ray_rs_core::models::{
    AppSettings, BackendType, DnsHijackMode, TunStack, validate_domain_pattern, validate_ip_cidr,
    validate_tun_interface_name,
};

use super::{SettingsCallback, SettingsObservers, ToastCallback, subscribe_settings};

type RenderFn = Rc<dyn Fn()>;

pub(super) fn build_tun_page(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    observers: &SettingsObservers,
    toast_cb: &ToastCallback,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("TUN")
        .icon_name("network-vpn-symbolic")
        .build();

    let backend = state.borrow().backend.backend_type;
    let is_v2ray = backend == BackendType::V2ray;

    // --- Primary group ------------------------------------------------------
    let primary = adw::PreferencesGroup::builder()
        .title("TUN mode")
        .description("Route all system traffic through the active proxy")
        .build();

    let enable_row = adw::SwitchRow::builder()
        .title("Enable TUN")
        .active(state.borrow().tun.enabled)
        .sensitive(!is_v2ray)
        .build();
    primary.add(&enable_row);

    let backend_note = adw::ActionRow::builder()
        .title("TUN requires sing-box or xray")
        .subtitle("v2ray-core has no native TUN inbound")
        .sensitive(false)
        .visible(is_v2ray)
        .build();
    primary.add(&backend_note);

    let iface_row = adw::EntryRow::builder()
        .title("Interface name")
        .text(&state.borrow().tun.interface_name)
        .show_apply_button(true)
        .build();
    primary.add(&iface_row);

    let address_row = adw::EntryRow::builder()
        .title("IPv4 address (CIDR)")
        .text(&state.borrow().tun.address_v4)
        .show_apply_button(true)
        .build();
    primary.add(&address_row);

    let mtu_row = adw::SpinRow::builder()
        .title("MTU")
        .adjustment(&gtk::Adjustment::new(
            state.borrow().tun.mtu as f64,
            576.0,
            9000.0,
            1.0,
            0.0,
            0.0,
        ))
        .build();
    primary.add(&mtu_row);
    page.add(&primary);

    // --- Advanced -----------------------------------------------------------
    let advanced_group = adw::PreferencesGroup::new();
    let advanced = adw::ExpanderRow::builder()
        .title("Advanced")
        .subtitle("Stack, routing, DNS, and exclusions")
        .build();

    let advanced_note = adw::ActionRow::builder()
        .title("These options apply to sing-box only")
        .sensitive(false)
        .visible(backend == BackendType::Xray)
        .build();
    advanced.add_row(&advanced_note);

    let stack_row = adw::ComboRow::builder()
        .title("Stack")
        .model(&gtk::StringList::new(&["system", "gvisor", "mixed"]))
        .selected(stack_to_index(state.borrow().tun.stack))
        .build();
    advanced.add_row(&stack_row);

    let strict_row = adw::SwitchRow::builder()
        .title("Strict route")
        .active(state.borrow().tun.strict_route)
        .build();
    advanced.add_row(&strict_row);

    let hijack_row = adw::ComboRow::builder()
        .title("DNS hijack")
        .model(&gtk::StringList::new(&["hijack", "native", "disabled"]))
        .selected(hijack_to_index(state.borrow().tun.dns_hijack))
        .build();
    advanced.add_row(&hijack_row);

    advanced_group.add(&advanced);
    page.add(&advanced_group);

    // --- Excluded routes ----------------------------------------------------
    let routes_group = adw::PreferencesGroup::builder()
        .title("Excluded routes")
        .description("CIDRs that bypass the tunnel")
        .build();
    let add_row = adw::ActionRow::builder()
        .title("Add excluded route")
        .activatable(true)
        .build();
    add_row.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
    routes_group.add(&add_row);
    page.add(&routes_group);

    let added_rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let render_slot: Rc<RefCell<Option<RenderFn>>> = Rc::new(RefCell::new(None));
    let render_routes: RenderFn = {
        let routes_group = routes_group.clone();
        let state = state.clone();
        let cb = cb.clone();
        let added_rows = added_rows.clone();
        let render_slot = render_slot.clone();
        Rc::new(move || {
            for row in added_rows.borrow().iter() {
                routes_group.remove(row);
            }
            added_rows.borrow_mut().clear();
            let routes = state.borrow().tun.exclude_routes.clone();
            for (index, route) in routes.into_iter().enumerate() {
                let row = adw::ActionRow::builder().title(&route).build();
                let delete = gtk::Button::builder()
                    .icon_name("user-trash-symbolic")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                {
                    let state = state.clone();
                    let cb = cb.clone();
                    let render_slot = render_slot.clone();
                    delete.connect_clicked(move |_| {
                        let _ = apply_tun_mutation(&state, &cb, |s| {
                            if index < s.tun.exclude_routes.len() {
                                s.tun.exclude_routes.remove(index);
                            }
                            Ok(())
                        });
                        if let Some(render) = render_slot.borrow().as_ref() {
                            render();
                        }
                    });
                }
                row.add_suffix(&delete);
                routes_group.add(&row);
                added_rows.borrow_mut().push(row);
            }
        })
    };
    *render_slot.borrow_mut() = Some(render_routes.clone());
    render_routes();

    {
        let state = state.clone();
        let cb = cb.clone();
        let render_routes = render_routes.clone();
        add_row.connect_activated(move |_| {
            let dialog = adw::AlertDialog::builder()
                .heading("Add excluded route")
                .build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("add", "Add");
            dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("add"));
            dialog.set_close_response("cancel");

            let entry = adw::EntryRow::builder()
                .title("CIDR (e.g. 192.168.0.0/16)")
                .build();
            let group = adw::PreferencesGroup::new();
            group.add(&entry);
            dialog.set_extra_child(Some(&group));

            let state = state.clone();
            let cb = cb.clone();
            let render_routes = render_routes.clone();
            let entry = entry.clone();
            dialog.connect_response(Some("add"), move |_, _| {
                let value = entry.text().trim().to_string();
                if validate_ip_cidr(&value).is_ok() {
                    let _ = apply_tun_mutation(&state, &cb, |s| {
                        s.tun.exclude_routes.push(value.clone());
                        Ok(())
                    });
                    render_routes();
                }
            });
            dialog.present(crate::active_window().as_ref());
        });
    }

    // --- Excluded domains ---------------------------------------------------
    let domains_group = adw::PreferencesGroup::builder()
        .title("Excluded domains")
        .description("Domain suffixes that bypass the tunnel")
        .build();
    let add_domain_row = adw::ActionRow::builder()
        .title("Add excluded domain")
        .activatable(true)
        .build();
    add_domain_row.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
    domains_group.add(&add_domain_row);
    page.add(&domains_group);

    let domains_added_rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let domains_render_slot: Rc<RefCell<Option<RenderFn>>> = Rc::new(RefCell::new(None));
    let render_domains: RenderFn = {
        let domains_group = domains_group.clone();
        let state = state.clone();
        let cb = cb.clone();
        let added_rows = domains_added_rows.clone();
        let render_slot = domains_render_slot.clone();
        Rc::new(move || {
            for row in added_rows.borrow().iter() {
                domains_group.remove(row);
            }
            added_rows.borrow_mut().clear();
            let domains = state.borrow().tun.exclude_domains.clone();
            for (index, domain) in domains.into_iter().enumerate() {
                let row = adw::ActionRow::builder().title(&domain).build();
                let delete = gtk::Button::builder()
                    .icon_name("user-trash-symbolic")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                {
                    let state = state.clone();
                    let cb = cb.clone();
                    let render_slot = render_slot.clone();
                    delete.connect_clicked(move |_| {
                        let _ = apply_tun_mutation(&state, &cb, |s| {
                            if index < s.tun.exclude_domains.len() {
                                s.tun.exclude_domains.remove(index);
                            }
                            Ok(())
                        });
                        if let Some(render) = render_slot.borrow().as_ref() {
                            render();
                        }
                    });
                }
                row.add_suffix(&delete);
                domains_group.add(&row);
                added_rows.borrow_mut().push(row);
            }
        })
    };
    *domains_render_slot.borrow_mut() = Some(render_domains.clone());
    render_domains();

    {
        let state = state.clone();
        let cb = cb.clone();
        let render_domains = render_domains.clone();
        add_domain_row.connect_activated(move |_| {
            let dialog = adw::AlertDialog::builder()
                .heading("Add excluded domain")
                .build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("add", "Add");
            dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("add"));
            dialog.set_close_response("cancel");

            let entry = adw::EntryRow::builder()
                .title("Domain suffix (e.g. example.com)")
                .build();
            let group = adw::PreferencesGroup::new();
            group.add(&entry);
            dialog.set_extra_child(Some(&group));

            let state = state.clone();
            let cb = cb.clone();
            let render_domains = render_domains.clone();
            let entry = entry.clone();
            dialog.connect_response(Some("add"), move |_, _| {
                let value = entry.text().trim().to_string();
                if validate_domain_pattern(&value).is_ok() {
                    let _ = apply_tun_mutation(&state, &cb, |s| {
                        s.tun.exclude_domains.push(value.clone());
                        Ok(())
                    });
                    render_domains();
                }
            });
            dialog.present(crate::active_window().as_ref());
        });
    }

    // --- Excluded applications ----------------------------------------------
    let apps_group = adw::PreferencesGroup::builder()
        .title("Excluded applications")
        .description("Process names that bypass the tunnel (sing-box only)")
        .build();
    let apps_note = adw::ActionRow::builder()
        .title("xray cannot match TUN traffic by process name")
        .sensitive(false)
        .visible(backend == BackendType::Xray)
        .build();
    apps_group.add(&apps_note);
    let add_app_row = adw::ActionRow::builder()
        .title("Add excluded application")
        .activatable(true)
        .build();
    add_app_row.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
    apps_group.add(&add_app_row);
    page.add(&apps_group);

    let apps_added_rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let apps_render_slot: Rc<RefCell<Option<RenderFn>>> = Rc::new(RefCell::new(None));
    let render_apps: RenderFn = {
        let apps_group = apps_group.clone();
        let state = state.clone();
        let cb = cb.clone();
        let added_rows = apps_added_rows.clone();
        let render_slot = apps_render_slot.clone();
        Rc::new(move || {
            for row in added_rows.borrow().iter() {
                apps_group.remove(row);
            }
            added_rows.borrow_mut().clear();
            let processes = state.borrow().tun.exclude_processes.clone();
            for (index, process) in processes.into_iter().enumerate() {
                let row = adw::ActionRow::builder().title(&process).build();
                let delete = gtk::Button::builder()
                    .icon_name("user-trash-symbolic")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                {
                    let state = state.clone();
                    let cb = cb.clone();
                    let render_slot = render_slot.clone();
                    delete.connect_clicked(move |_| {
                        let _ = apply_tun_mutation(&state, &cb, |s| {
                            if index < s.tun.exclude_processes.len() {
                                s.tun.exclude_processes.remove(index);
                            }
                            Ok(())
                        });
                        if let Some(render) = render_slot.borrow().as_ref() {
                            render();
                        }
                    });
                }
                row.add_suffix(&delete);
                apps_group.add(&row);
                added_rows.borrow_mut().push(row);
            }
        })
    };
    *apps_render_slot.borrow_mut() = Some(render_apps.clone());
    render_apps();

    {
        let state = state.clone();
        let cb = cb.clone();
        let render_apps = render_apps.clone();
        add_app_row.connect_activated(move |_| {
            let dialog = adw::AlertDialog::builder()
                .heading("Add excluded application")
                .build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("add", "Add");
            dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("add"));
            dialog.set_close_response("cancel");

            let entry = adw::EntryRow::builder()
                .title("Process name (e.g. cloudflared)")
                .build();
            let group = adw::PreferencesGroup::new();
            group.add(&entry);
            dialog.set_extra_child(Some(&group));

            let state = state.clone();
            let cb = cb.clone();
            let render_apps = render_apps.clone();
            let entry = entry.clone();
            dialog.connect_response(Some("add"), move |_, _| {
                let value = entry.text().trim().to_string();
                if !value.is_empty() && !value.contains('/') && !value.contains('\\') {
                    let _ = apply_tun_mutation(&state, &cb, |s| {
                        s.tun.exclude_processes.push(value.clone());
                        Ok(())
                    });
                    render_apps();
                }
            });
            dialog.present(crate::active_window().as_ref());
        });
    }

    // --- Run with bypass (xray only) ----------------------------------------
    let run_group = adw::PreferencesGroup::builder()
        .title("Run with bypass")
        .description("Launch a command routed outside the xray TUN")
        .visible(backend == BackendType::Xray)
        .build();

    let run_bin = v2ray_rs_process::run_path();
    let wrapper_available = run_bin.is_file()
        || std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .any(|dir| dir.join(&run_bin).is_file());

    let run_note = adw::ActionRow::builder()
        .title("Wrapper not found")
        .subtitle("Install v2ray-rs-run and grant privileges first")
        .visible(!wrapper_available)
        .build();
    run_group.add(&run_note);

    let run_entry = adw::EntryRow::builder()
        .title("Command (e.g. cloudflared tunnel run)")
        .sensitive(wrapper_available)
        .build();
    let launch_button = gtk::Button::builder()
        .label("Launch")
        .valign(gtk::Align::Center)
        .sensitive(wrapper_available)
        .css_classes(["suggested-action"])
        .build();
    run_entry.add_suffix(&launch_button);
    run_group.add(&run_entry);
    page.add(&run_group);

    {
        let toast_cb = toast_cb.clone();
        let run_entry = run_entry.clone();
        launch_button.connect_clicked(move |_| {
            let text = run_entry.text().trim().to_string();
            if text.is_empty() {
                return;
            }
            let mut cmd = tokio::process::Command::new(v2ray_rs_process::run_path());
            cmd.args(text.split_whitespace());
            match cmd.spawn() {
                Ok(mut child) => {
                    tokio::spawn(async move {
                        let _ = child.wait().await;
                    });
                    toast_cb("Launched via bypass wrapper.");
                }
                Err(err) => toast_cb(&format!("Launch failed: {err}")),
            }
        });
    }

    // --- Capabilities -------------------------------------------------------
    let cap_group = adw::PreferencesGroup::builder().title("Privileges").build();
    let cap_row = adw::ActionRow::builder().title("Capability status").build();
    let grant_button = gtk::Button::builder()
        .label("Grant TUN privileges")
        .valign(gtk::Align::Center)
        .build();
    cap_row.add_suffix(&grant_button);
    cap_group.add(&cap_row);
    page.add(&cap_group);

    let refresh_caps: RenderFn = {
        let state = state.clone();
        let cap_row = cap_row.clone();
        let grant_button = grant_button.clone();
        Rc::new(move || {
            let path = state.borrow().backend.binary_path.clone();
            match path {
                None => {
                    cap_row.set_subtitle("Configure a backend binary first");
                    grant_button.set_visible(false);
                }
                Some(path) => match v2ray_rs_process::has_net_admin(&path) {
                    Ok(true) => {
                        cap_row.set_subtitle("CAP_NET_ADMIN granted");
                        grant_button.set_visible(false);
                    }
                    Ok(false) => {
                        cap_row.set_subtitle("Backend lacks CAP_NET_ADMIN");
                        grant_button.set_visible(true);
                    }
                    Err(err) => {
                        cap_row.set_subtitle(&format!("Cannot read capabilities: {err}"));
                        grant_button.set_visible(true);
                    }
                },
            }
        })
    };
    refresh_caps();

    {
        let state = state.clone();
        let refresh = refresh_caps.clone();
        let toast_cb = toast_cb.clone();
        grant_button.connect_clicked(move |_| {
            let backend_path = state.borrow().backend.binary_path.clone();
            let Some(backend_path) = backend_path else {
                return;
            };
            let helper = v2ray_rs_process::helper_path();
            match v2ray_rs_process::grant(&backend_path, &helper) {
                Ok(()) => toast_cb("TUN privileges granted."),
                Err(err) => toast_cb(&format!("Grant failed: {err}")),
            }
            refresh();
        });
    }

    // --- Handlers -----------------------------------------------------------
    {
        let state = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        let warned = Rc::new(Cell::new(false));
        enable_row.connect_active_notify(move |row| {
            let enabled = row.is_active();
            let _ = apply_tun_mutation(&state, &cb, |s| {
                s.tun.enabled = enabled;
                Ok(())
            });
            if enabled && !warned.replace(true) {
                toast_cb("TUN routes all system traffic through the active proxy.");
            }
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        iface_row.connect_apply(move |row| {
            let value = row.text().trim().to_string();
            match validate_tun_interface_name(&value) {
                Ok(()) => {
                    row.remove_css_class("error");
                    let _ = apply_tun_mutation(&state, &cb, |s| {
                        s.tun.interface_name = value.clone();
                        Ok(())
                    });
                }
                Err(_) => {
                    row.add_css_class("error");
                    row.set_text(&state.borrow().tun.interface_name);
                }
            }
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        address_row.connect_apply(move |row| {
            let value = row.text().trim().to_string();
            match validate_ip_cidr(&value) {
                Ok(()) => {
                    row.remove_css_class("error");
                    let _ = apply_tun_mutation(&state, &cb, |s| {
                        s.tun.address_v4 = value.clone();
                        Ok(())
                    });
                }
                Err(_) => {
                    row.add_css_class("error");
                    row.set_text(&state.borrow().tun.address_v4);
                }
            }
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        mtu_row.connect_changed(move |row| {
            let mtu = row.value() as u16;
            let _ = apply_tun_mutation(&state, &cb, |s| {
                s.tun.mtu = mtu;
                Ok(())
            });
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        stack_row.connect_selected_notify(move |row| {
            let stack = index_to_stack(row.selected());
            let _ = apply_tun_mutation(&state, &cb, |s| {
                s.tun.stack = stack;
                Ok(())
            });
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        strict_row.connect_active_notify(move |row| {
            let active = row.is_active();
            let _ = apply_tun_mutation(&state, &cb, |s| {
                s.tun.strict_route = active;
                Ok(())
            });
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        hijack_row.connect_selected_notify(move |row| {
            let mode = index_to_hijack(row.selected());
            let _ = apply_tun_mutation(&state, &cb, |s| {
                s.tun.dns_hijack = mode;
                Ok(())
            });
        });
    }

    // --- Reactive backend / capability gating -------------------------------
    {
        let enable_row = enable_row.clone();
        let backend_note = backend_note.clone();
        let advanced_note = advanced_note.clone();
        let apps_note = apps_note.clone();
        let stack_row = stack_row.clone();
        let strict_row = strict_row.clone();
        let hijack_row = hijack_row.clone();
        let routes_group = routes_group.clone();
        let domains_group = domains_group.clone();
        let run_group = run_group.clone();
        let apps_group = apps_group.clone();
        let refresh_caps = refresh_caps.clone();
        subscribe_settings(observers, move |settings| {
            let backend = settings.backend.backend_type;
            let singbox_only = backend == BackendType::SingBox;
            enable_row.set_sensitive(backend != BackendType::V2ray);
            backend_note.set_visible(backend == BackendType::V2ray);
            advanced_note.set_visible(backend == BackendType::Xray);
            run_group.set_visible(backend == BackendType::Xray);
            apps_note.set_visible(backend == BackendType::Xray);
            stack_row.set_sensitive(singbox_only);
            strict_row.set_sensitive(singbox_only);
            hijack_row.set_sensitive(singbox_only);
            routes_group.set_sensitive(backend != BackendType::V2ray);
            domains_group.set_sensitive(backend != BackendType::V2ray);
            apps_group.set_sensitive(singbox_only);
            refresh_caps();
        });
    }
    // Apply the initial backend gating for the advanced rows.
    let singbox_only = backend == BackendType::SingBox;
    stack_row.set_sensitive(singbox_only);
    strict_row.set_sensitive(singbox_only);
    hijack_row.set_sensitive(singbox_only);
    routes_group.set_sensitive(backend != BackendType::V2ray);
    domains_group.set_sensitive(backend != BackendType::V2ray);
    apps_group.set_sensitive(singbox_only);

    page
}

fn apply_tun_mutation<F>(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    mutate: F,
) -> Result<(), String>
where
    F: FnOnce(&mut AppSettings) -> Result<(), String>,
{
    let mut next = state.borrow().clone();
    mutate(&mut next)?;
    next.tun.validate().map_err(|e| e.to_string())?;
    *state.borrow_mut() = next.clone();
    cb(next);
    Ok(())
}

fn stack_to_index(stack: TunStack) -> u32 {
    match stack {
        TunStack::System => 0,
        TunStack::Gvisor => 1,
        TunStack::Mixed => 2,
    }
}

fn index_to_stack(index: u32) -> TunStack {
    match index {
        1 => TunStack::Gvisor,
        2 => TunStack::Mixed,
        _ => TunStack::System,
    }
}

fn hijack_to_index(mode: DnsHijackMode) -> u32 {
    match mode {
        DnsHijackMode::Hijack => 0,
        DnsHijackMode::Native => 1,
        DnsHijackMode::Disabled => 2,
    }
}

fn index_to_hijack(index: u32) -> DnsHijackMode {
    match index {
        1 => DnsHijackMode::Native,
        2 => DnsHijackMode::Disabled,
        _ => DnsHijackMode::Hijack,
    }
}
