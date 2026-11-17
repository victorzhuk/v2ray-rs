use super::*;
pub(super) fn upsert_dns_rule(
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

pub(super) fn dns_rule_match_validation_error(m: &DnsRuleMatch) -> Option<String> {
    match m {
        DnsRuleMatch::DomainKeyword { keyword } => validate_domain_keyword(keyword)
            .err()
            .map(|e| e.to_string()),
        _ => None,
    }
}

pub(super) fn dns_rule_from_inputs(
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
        2 => DnsRuleMatch::DomainKeyword { keyword: value },
        3 => DnsRuleMatch::DomainFull { domain: value },
        _ => DnsRuleMatch::GeoSite { category: value },
    };
    if let Some(msg) = dns_rule_match_validation_error(&match_condition) {
        return Err(msg);
    }

    let server_idx = server_combo.selected() as usize;
    let Some(server_tag) = servers.get(server_idx) else {
        return Err("Server tag is required".to_string());
    };

    Ok(DnsRule {
        match_condition,
        server_tag: server_tag.clone(),
    })
}

pub(super) fn render_dns_rules(ctx: &DnsRenderCtx) {
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
            DnsRuleMatch::DomainKeyword { keyword } => ("Domain Keyword", keyword),
            DnsRuleMatch::DomainFull { domain } => ("Domain Full", domain),
        };

        let row = adw::ActionRow::builder()
            .title(format!("{match_type}: {value}"))
            .build();
        if let Some(msg) = dns_rule_match_validation_error(&rule.match_condition) {
            row.add_css_class("error");
            row.set_subtitle(&msg);
        } else {
            row.set_subtitle(&format!("Server: {}", rule.server_tag));
        }

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

pub(super) fn show_dns_rule_dialog(existing: Option<DnsRule>, ctx: &DnsRenderCtx) {
    let is_edit = existing.is_some();

    let (init_type, init_value, init_server_tag) = match &existing {
        Some(rule) => {
            let (type_idx, val) = match &rule.match_condition {
                DnsRuleMatch::GeoSite { category } => (0u32, category.clone()),
                DnsRuleMatch::DomainSuffix { suffix } => (1, suffix.clone()),
                DnsRuleMatch::DomainKeyword { keyword } => (2, keyword.clone()),
                DnsRuleMatch::DomainFull { domain } => (3, domain.clone()),
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
            "Domain Keyword",
            "Domain Full",
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
