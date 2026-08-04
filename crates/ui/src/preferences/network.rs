use adw::prelude::*;
use relm4::adw;
use relm4::gtk;
use relm4::gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use v2ray_rs_core::backend::{DetectedBackend, detect_all, validate_custom_path};
use v2ray_rs_core::geodata::GeodataManager;
use v2ray_rs_core::geodata_index::GeodataIndexManager;
use v2ray_rs_core::models::{AppSettings, AutoResolveStrategy, BackendConfig, BackendType};
use v2ray_rs_core::persistence::AppPaths;

use super::{
    SettingsCallback, SettingsObservers, ToastCallback, clear_preferences_group,
    current_backend_status, emit, render_detected_backends, set_current_backend_status,
    subscribe_settings,
};

pub(super) fn build_network_page(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    settings_observers: &SettingsObservers,
    paths: &AppPaths,
    toast_cb: &ToastCallback,
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
    let detected_state = Rc::new(RefCell::new(Vec::<DetectedBackend>::new()));
    let backend_loading_row = adw::ActionRow::builder()
        .title("Scanning installed backends")
        .subtitle("Checking common paths and backend version output")
        .activatable(false)
        .build();
    let backend_loading_spinner = gtk::Spinner::builder()
        .spinning(true)
        .valign(gtk::Align::Center)
        .build();
    backend_loading_row.add_suffix(&backend_loading_spinner);
    backend_group.add(&backend_loading_row);
    page.add(&backend_group);

    let custom_group = adw::PreferencesGroup::builder()
        .title("Custom Backend Path")
        .description("Validate and use a manually provided backend binary")
        .build();

    let custom_type_row = adw::ComboRow::builder()
        .title("Backend type")
        .model(&gtk::StringList::new(&["v2ray", "xray", "sing-box"]))
        .selected(s.backend.backend_type.to_index())
        .build();
    custom_group.add(&custom_type_row);

    let custom_path_row = adw::EntryRow::builder()
        .title("Binary path")
        .text(
            s.backend
                .binary_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        )
        .build();
    custom_group.add(&custom_path_row);

    let custom_status_row = adw::ActionRow::builder()
        .title("Custom path status")
        .subtitle(current_backend_status(&s, &[]))
        .build();
    custom_group.add(&custom_status_row);

    let validate_custom_row = adw::ActionRow::builder().title("Custom path").build();
    let validate_custom_spinner = gtk::Spinner::builder()
        .visible(false)
        .valign(gtk::Align::Center)
        .build();
    let validate_custom_btn = gtk::Button::builder()
        .label("Validate & Use")
        .css_classes(["suggested-action"])
        .build();
    validate_custom_row.add_suffix(&validate_custom_spinner);
    validate_custom_row.add_suffix(&validate_custom_btn);
    custom_group.add(&validate_custom_row);
    page.add(&custom_group);

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

    let idle_timeout_row = adw::SpinRow::builder()
        .title("Idle connection timeout (seconds)")
        .subtitle("Streams idle longer than this are closed by the backend")
        .adjustment(&gtk::Adjustment::new(
            s.idle_timeout_secs as f64,
            60.0,
            86400.0,
            30.0,
            0.0,
            0.0,
        ))
        .build();
    ports_group.add(&idle_timeout_row);

    let listen_address_row = adw::EntryRow::builder()
        .title("Listen address")
        .text(s.listen_address.as_str())
        .build();
    let listen_address_status = adw::ActionRow::builder()
        .title("Listen address status")
        .subtitle(listen_address_status_text(&s.listen_address))
        .build();
    ports_group.add(&listen_address_row);
    ports_group.add(&listen_address_status);
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

    let index_manager = GeodataIndexManager::new(paths);

    let geodata_status = geodata_status_text(&index_manager, backend_type);

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
            let paths_for_status = paths.clone();
            let paths_for_task = paths.clone();
            let index_manager = GeodataIndexManager::new(&paths);

            let btn_clone = btn.clone();
            let spinner_clone = spinner.clone();
            let status_row_clone = status_row.clone();

            glib::MainContext::default().spawn_local(async move {
                let result = tokio::task::spawn_blocking(move || -> Result<String, String> {
                    #[cfg(feature = "geodata-fetch")]
                    {
                        use v2ray_rs_core::geodata::{
                            download_geodata, download_singbox_rule_sets,
                        };
                        use v2ray_rs_core::persistence::{load_routing_rules, load_subscriptions};

                        if backend_type == BackendType::SingBox {
                            let rules = load_routing_rules(&paths_for_task).unwrap_or_default();
                            let subscriptions =
                                load_subscriptions(&paths_for_task).unwrap_or_default();
                            let tags = crate::geodata_service::singbox_rule_set_tags(
                                &rules,
                                &subscriptions,
                            );
                            let missing: Vec<String> = tags
                                .into_iter()
                                .filter(|tag| !geodata_manager.has_rule_set(tag))
                                .collect();

                            if !missing.is_empty() {
                                download_singbox_rule_sets(&geodata_manager, &missing)
                                    .map_err(|e| format!("Download failed: {}", e))?;
                            }

                            index_manager
                                .build_singbox_index(&geodata_manager.rule_sets_dir())
                                .map_err(|e| format!("Index build failed: {}", e))?;
                        } else {
                            download_geodata(&geodata_manager)
                                .map_err(|e| format!("Download failed: {}", e))?;

                            let geoip_path = geodata_manager.geoip_path();
                            let geosite_path = geodata_manager.geosite_path();

                            index_manager
                                .build_index(backend_type, &geoip_path, &geosite_path)
                                .map_err(|e| format!("Index build failed: {}", e))?;
                        }

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
                        if let Ok(Some(index)) =
                            GeodataIndexManager::new(&paths_for_status).load_index(backend_type)
                        {
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
            AutoResolveStrategy::LastSuccessful => 3,
        })
        .build();
    resolve_group.add(&resolve_row);
    page.add(&resolve_group);

    let real_delay_group = adw::PreferencesGroup::builder()
        .title("Real Delay")
        .description("End-to-end latency probe through an ephemeral backend instance")
        .build();

    let real_delay_enabled_row = adw::SwitchRow::builder()
        .title("Enabled")
        .active(s.real_delay.enabled)
        .build();
    real_delay_group.add(&real_delay_enabled_row);

    let real_delay_url_row = adw::EntryRow::builder()
        .title("Test URL")
        .text(&s.real_delay.test_url)
        .show_apply_button(true)
        .build();
    real_delay_group.add(&real_delay_url_row);

    let real_delay_timeout_row = adw::SpinRow::builder()
        .title("Timeout (ms)")
        .adjustment(&gtk::Adjustment::new(
            s.real_delay.timeout_ms as f64,
            500.0,
            60000.0,
            500.0,
            0.0,
            0.0,
        ))
        .build();
    real_delay_group.add(&real_delay_timeout_row);

    let real_delay_use_for_lowest_row = adw::SwitchRow::builder()
        .title("Use for Lowest Latency strategy")
        .subtitle("Prefer real delay over TCP ping when sorting by latency")
        .active(s.real_delay.use_for_lowest_latency)
        .build();
    real_delay_group.add(&real_delay_use_for_lowest_row);

    let real_delay_preset_row = adw::ComboRow::builder()
        .title("Test URL Preset")
        .model(&gtk::StringList::new(&[
            "Google (gstatic.com)",
            "Cloudflare",
            "Apple",
        ]))
        .build();
    real_delay_group.add(&real_delay_preset_row);

    page.add(&real_delay_group);

    drop(s);

    {
        let backend_group = backend_group.clone();
        let st = state.clone();
        let cb = cb.clone();
        let detected_state = detected_state.clone();
        let custom_status_row = custom_status_row.clone();
        glib::MainContext::default().spawn_local(async move {
            match tokio::task::spawn_blocking(detect_all).await {
                Ok(detected) => {
                    *detected_state.borrow_mut() = detected.clone();
                    render_detected_backends(
                        &backend_group,
                        &detected,
                        &st,
                        &cb,
                        &detected_state,
                        &custom_status_row,
                    );
                    let settings = st.borrow();
                    set_current_backend_status(&custom_status_row, &settings, &detected);
                }
                Err(err) => {
                    clear_preferences_group(&backend_group);
                    let row = adw::ActionRow::builder()
                        .title("Backend scan failed")
                        .subtitle(err.to_string())
                        .sensitive(false)
                        .build();
                    backend_group.add(&row);
                    custom_status_row
                        .set_subtitle("Backend scan failed. You can still validate a custom path.");
                }
            }
        });
    }

    {
        let st = state.clone();
        let cb = cb.clone();
        let custom_type_row = custom_type_row.clone();
        let custom_path_row = custom_path_row.clone();
        let custom_status_row = custom_status_row.clone();
        let validate_custom_btn = validate_custom_btn.clone();
        let validate_custom_spinner = validate_custom_spinner.clone();
        let detected_state = detected_state.clone();
        let validation_generation = Rc::new(Cell::new(0_u64));
        validate_custom_btn.clone().connect_clicked(move |_| {
            let path_text = custom_path_row.text().trim().to_string();
            if path_text.is_empty() {
                custom_status_row.set_subtitle("Enter a path to validate");
                return;
            }

            let backend_type =
                BackendType::from_index(custom_type_row.selected()).unwrap_or(BackendType::Xray);
            let request_id = validation_generation.get().wrapping_add(1);
            validation_generation.set(request_id);
            validate_custom_btn.set_sensitive(false);
            validate_custom_spinner.set_visible(true);
            validate_custom_spinner.start();
            custom_status_row.set_subtitle("Validating custom backend path...");

            let st = st.clone();
            let cb = cb.clone();
            let custom_status_row = custom_status_row.clone();
            let validate_custom_btn = validate_custom_btn.clone();
            let validate_custom_spinner = validate_custom_spinner.clone();
            let detected_state = detected_state.clone();
            let validation_generation = validation_generation.clone();
            glib::MainContext::default().spawn_local(async move {
                let path = std::path::PathBuf::from(path_text);
                let result = tokio::task::spawn_blocking(move || {
                    validate_custom_path(&path, backend_type).map_err(|err| err.to_string())
                })
                .await;

                if validation_generation.get() != request_id {
                    return;
                }

                validate_custom_spinner.stop();
                validate_custom_spinner.set_visible(false);
                validate_custom_btn.set_sensitive(true);

                match result {
                    Ok(Ok(detected_backend)) => {
                        let mut settings = st.borrow_mut();
                        settings.backend = BackendConfig {
                            backend_type: detected_backend.backend_type,
                            binary_path: Some(detected_backend.binary_path),
                            config_output_dir: settings.backend.config_output_dir.clone(),
                        };
                        let detected = detected_state.borrow();
                        set_current_backend_status(&custom_status_row, &settings, &detected);
                        drop(detected);
                        drop(settings);
                        emit(&st, &cb);
                    }
                    Ok(Err(err)) => {
                        custom_status_row.set_subtitle(&err);
                    }
                    Err(err) => {
                        custom_status_row.set_subtitle(&format!("Validation task failed: {err}"));
                    }
                }
            });
        });
    }
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
        idle_timeout_row.connect_changed(move |row| {
            st.borrow_mut().idle_timeout_secs = row.value() as u32;
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        let status_row = listen_address_status.clone();
        let toast_cb = toast_cb.clone();
        listen_address_row.connect_apply(move |row| {
            let value = row.text().trim().to_string();
            match AppSettings::validate_listen_address(&value) {
                Ok(()) => {
                    row.remove_css_class("error");
                    let previous = st.borrow().listen_address.clone();
                    st.borrow_mut().listen_address = value.clone();
                    status_row.set_subtitle(&listen_address_status_text(&value));
                    emit(&st, &cb);
                    if previous != value && !is_loopback_listen_address(&value) {
                        toast_cb("Proxy now reachable from other hosts on this network.");
                    }
                }
                Err(err) => {
                    row.add_css_class("error");
                    status_row.set_subtitle(&format!("Invalid: {err}"));
                    // Reset entry to the last known good value.
                    let current = st.borrow().listen_address.clone();
                    row.set_text(&current);
                }
            }
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
    {
        let st = state.clone();
        let cb = cb.clone();
        real_delay_enabled_row.connect_active_notify(move |row| {
            st.borrow_mut().real_delay.enabled = row.is_active();
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        real_delay_url_row.connect_apply(move |row| {
            let value = row.text().trim().to_string();
            match AppSettings::validate_real_delay_url(&value) {
                Ok(()) => {
                    row.remove_css_class("error");
                    st.borrow_mut().real_delay.test_url = value;
                    emit(&st, &cb);
                }
                Err(err) => {
                    row.add_css_class("error");
                    toast_cb(&format!("Invalid Real Delay test URL: {err}"));
                    let current = st.borrow().real_delay.test_url.clone();
                    row.set_text(&current);
                }
            }
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        real_delay_timeout_row.connect_changed(move |row| {
            st.borrow_mut().real_delay.timeout_ms = row.value() as u32;
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        real_delay_use_for_lowest_row.connect_active_notify(move |row| {
            st.borrow_mut().real_delay.use_for_lowest_latency = row.is_active();
            emit(&st, &cb);
        });
    }
    {
        let st = state.clone();
        let cb = cb.clone();
        let real_delay_url_row = real_delay_url_row.clone();
        real_delay_preset_row.connect_selected_notify(move |row| {
            let Some(url) = real_delay_preset_url(row.selected()) else {
                return;
            };
            real_delay_url_row.set_text(url);
            st.borrow_mut().real_delay.test_url = url.to_string();
            emit(&st, &cb);
        });
    }
    {
        let custom_type_row = custom_type_row.clone();
        let custom_status_row = custom_status_row.clone();
        let geodata_row = geodata_row.clone();
        let detected_state = detected_state.clone();
        let paths = paths.clone();
        subscribe_settings(settings_observers, move |settings| {
            custom_type_row.set_selected(settings.backend.backend_type.to_index());

            let detected = detected_state.borrow();
            set_current_backend_status(&custom_status_row, settings, &detected);
            drop(detected);

            let index_manager = GeodataIndexManager::new(&paths);
            geodata_row.set_subtitle(&geodata_status_text(
                &index_manager,
                settings.backend.backend_type,
            ));
        });
    }

    page
}

fn is_loopback_listen_address(addr: &str) -> bool {
    matches!(addr, "127.0.0.1" | "::1")
}

fn listen_address_status_text(addr: &str) -> String {
    if is_loopback_listen_address(addr) {
        "Loopback only (default). Proxy reachable from this machine only.".to_string()
    } else if AppSettings::validate_listen_address(addr).is_ok() {
        "Warning: non-loopback bind. The inbound proxy has no authentication and \
         will accept connections from any host on this network."
            .to_string()
    } else {
        format!("Invalid: {addr}")
    }
}

fn geodata_status_text(index_manager: &GeodataIndexManager, backend_type: BackendType) -> String {
    match index_manager.load_index(backend_type) {
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
    }
}

fn real_delay_preset_url(index: u32) -> Option<&'static str> {
    match index {
        0 => Some("https://www.gstatic.com/generate_204"),
        1 => Some("https://cp.cloudflare.com/generate_204"),
        2 => Some("https://www.apple.com/library/test/success.html"),
        _ => None,
    }
}
