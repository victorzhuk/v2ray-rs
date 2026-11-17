use super::*;
pub(super) fn server_row_subtitle(
    server: &DnsServerConfig,
    backend: BackendType,
    tun_enabled: bool,
) -> String {
    let mut subtitle = format!(
        "{}://{}:{}",
        format!("{:?}", server.protocol).to_lowercase(),
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

    if let Some(warning) = private_dns_warning(server, backend, tun_enabled) {
        subtitle.push('\n');
        subtitle.push_str(warning);
    }

    subtitle
}

pub(super) fn upsert_dns_server(
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

pub(super) fn dns_server_from_inputs(
    tag_entry: &adw::EntryRow,
    protocol_combo: &adw::ComboRow,
    address_entry: &adw::EntryRow,
    port_spin: &adw::SpinRow,
    honors_detour: bool,
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

    let detour = if honors_detour {
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

pub(super) fn render_dns_servers(ctx: &DnsRenderCtx) {
    for row in ctx.added_servers.borrow().iter() {
        ctx.servers_group.remove(row);
    }
    ctx.added_servers.borrow_mut().clear();

    let servers = ctx.state.borrow().dns.servers.clone();
    let backend = ctx.state.borrow().backend.backend_type;
    let tun_enabled = ctx.state.borrow().tun.enabled;

    let mut added = ctx.added_servers.borrow_mut();
    for server in &servers {
        let subtitle = server_row_subtitle(server, backend, tun_enabled);

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

pub(super) fn show_dns_remove_server_dialog(tag: String, ctx: &DnsRenderCtx) {
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

pub(super) fn show_dns_server_dialog(existing: Option<DnsServerConfig>, ctx: &DnsRenderCtx) {
    let is_edit = existing.is_some();

    let (init_tag, init_protocol, init_address, init_port, init_detour) = match &existing {
        Some(srv) => (
            srv.tag.clone(),
            protocol_to_index(srv.protocol),
            srv.address.clone(),
            srv.port.unwrap_or_default(),
            srv.detour.clone().unwrap_or_default(),
        ),
        None => (
            String::new(),
            0,
            String::new(),
            DnsProtocol::Udp.default_port(),
            String::new(),
        ),
    };

    let backend = ctx.state.borrow().backend.backend_type;
    // sing-box emits `detour` on the server; xray expresses a direct detour as
    // a routing rule. v2ray has neither.
    let honors_detour = matches!(backend, BackendType::SingBox | BackendType::Xray);

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
        .visible(honors_detour)
        .model(&gtk::StringList::new(&["proxy", "direct"]))
        .selected(if init_detour == "direct" { 1 } else { 0 })
        .build();
    if let Some(note) = detour_note(backend) {
        detour_combo.set_subtitle(note);
    }
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

    let private_warning_label = gtk::Label::builder()
        .label("")
        .wrap(true)
        .xalign(0.0)
        .halign(gtk::Align::Start)
        .visible(false)
        .build();

    content.append(&group);
    content.append(&error_label);
    content.append(&warning_label);
    content.append(&private_warning_label);
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

    let update_private_warning: Rc<dyn Fn()> = Rc::new({
        let state = ctx.state.clone();
        let private_warning_label = private_warning_label.clone();
        let protocol_combo = protocol_combo.clone();
        let address_entry = address_entry.clone();
        let detour_combo = detour_combo.clone();
        move || {
            let (backend, tun_enabled) = {
                let state = state.borrow();
                (state.backend.backend_type, state.tun.enabled)
            };
            let server = DnsServerConfig {
                tag: String::new(),
                protocol: index_to_protocol(protocol_combo.selected()),
                address: address_entry.text().to_string(),
                port: None,
                detour: Some(["proxy", "direct"][detour_combo.selected() as usize].to_string()),
            };
            match private_dns_warning(&server, backend, tun_enabled) {
                Some(warning) => {
                    private_warning_label.set_text(warning);
                    private_warning_label.set_visible(true);
                }
                None => {
                    private_warning_label.set_text("");
                    private_warning_label.set_visible(false);
                }
            }
        }
    });
    update_private_warning();
    {
        let update_private_warning = update_private_warning.clone();
        address_entry.connect_changed(move |_| update_private_warning());
    }
    {
        let update_private_warning = update_private_warning.clone();
        protocol_combo.connect_selected_notify(move |_| update_private_warning());
    }
    {
        let update_private_warning = update_private_warning.clone();
        detour_combo.connect_selected_notify(move |_| update_private_warning());
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
                honors_detour,
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
        port_spin.connect_changed(move |_| update_validation());
    }
    {
        let port_spin = port_spin.clone();
        let previous_default = Cell::new(index_to_protocol(init_protocol).default_port());
        protocol_combo.connect_selected_notify(move |row| {
            let next_default = index_to_protocol(row.selected()).default_port();
            if port_spin.value() as u16 == previous_default.get() {
                port_spin.set_value(next_default as f64);
            }
            previous_default.set(next_default);
        });
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
