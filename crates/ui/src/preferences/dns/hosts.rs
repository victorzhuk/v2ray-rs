use super::*;
pub(super) fn upsert_dns_host(
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

pub(super) fn dns_host_from_inputs(
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

pub(super) fn render_dns_hosts(ctx: &DnsRenderCtx) {
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
        let ip = host.ip.clone();
        remove_btn.connect_clicked(move |_| {
            if apply_dns_mutation(&ctx_clone, |settings| {
                settings
                    .dns
                    .hosts
                    .retain(|h| !(h.domain == domain && h.ip == ip));
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

pub(super) fn show_dns_host_dialog(existing: Option<HostOverride>, ctx: &DnsRenderCtx) {
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
