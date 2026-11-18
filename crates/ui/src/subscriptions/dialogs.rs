use super::*;
/// Opens the availability results view: probed nodes against the built-in
/// services, one badge per verdict with its checked-at time, and the node's
/// last known egress next to blocked verdicts.
pub(super) fn show_availability_dialog(
    sub: &Subscription,
    verdicts: &HashMap<(Uuid, Uuid), HashMap<String, VerdictRecord>>,
    egress_samples: &HashMap<(Uuid, Uuid), EgressSample>,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(format!("Availability — {}", sub.name))
        .body("Verdicts are measured through each node's own tunnel without touching the active connection")
        .build();
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();

    let services = builtin_service_checks();
    let mut rendered = 0usize;
    for node in &sub.nodes {
        let Some(node_verdicts) = verdicts.get(&(sub.id, node.id)) else {
            continue;
        };
        let group = adw::PreferencesGroup::builder()
            .title(node.node.remark().unwrap_or("Unnamed Node"))
            .build();
        let mut group_rows = 0usize;
        for service in services {
            let Some(record) = node_verdicts.get(service.id) else {
                continue;
            };
            let checked_at = record
                .checked_at
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string();
            let mut subtitle = format!("Checked: {checked_at}");
            if record.result == AvailabilityVerdict::Blocked
                && let Some(egress) = egress_samples.get(&(sub.id, node.id))
            {
                subtitle.push_str(&format!(" · {}", format_blocked_hint(egress)));
            }
            let row = adw::ActionRow::builder()
                .title(service.name)
                .subtitle(subtitle)
                .build();
            let badge = gtk::Label::builder()
                .label(record.result.label())
                .valign(gtk::Align::Center)
                .build();
            for class in verdict_css_classes(record.result) {
                badge.add_css_class(class);
            }
            row.add_suffix(&badge);
            group.add(&row);
            group_rows += 1;
            rendered += 1;
        }
        if group_rows > 0 {
            content.append(&group);
        }
    }
    if rendered == 0 {
        let empty = gtk::Label::builder()
            .label("No availability verdicts recorded yet. Run Check Availability first.")
            .css_classes(["dim-label"])
            .build();
        content.append(&empty);
    }

    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(200)
        .max_content_height(420)
        .propagate_natural_width(true)
        .child(&content)
        .build();
    dialog.set_extra_child(Some(&scroll));
    dialog.present(crate::active_window().as_ref());
}

pub(super) fn show_add_dialog(sender: ComponentSender<SubscriptionsPage>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Add Subscription")
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add");
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    dialog.set_close_response("cancel");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let name_entry = adw::EntryRow::builder().title("Name").build();
    let url_entry = adw::EntryRow::builder().title("URL").build();

    let group = adw::PreferencesGroup::new();
    group.add(&name_entry);
    group.add(&url_entry);
    let file_entry = adw::EntryRow::builder().title("Local File Path").build();
    group.add(&file_entry);
    content.append(&group);

    let paste_label = gtk::Label::builder()
        .label("Or paste a config-bundle JSON subscription")
        .halign(gtk::Align::Start)
        .css_classes(["caption"])
        .build();
    content.append(&paste_label);

    let paste_buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
    let paste_view = gtk::TextView::builder()
        .buffer(&paste_buffer)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(6)
        .right_margin(6)
        .build();
    let paste_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(96)
        .max_content_height(160)
        .child(&paste_view)
        .css_classes(["card"])
        .build();
    content.append(&paste_scroll);

    let error_label = gtk::Label::builder()
        .css_classes(["error"])
        .halign(gtk::Align::Start)
        .wrap(true)
        .visible(false)
        .build();
    content.append(&error_label);

    dialog.set_extra_child(Some(&content));

    {
        let dialog = dialog.clone();
        let update_validity = {
            let dialog = dialog.clone();
            let error_label = error_label.clone();
            let name_entry = name_entry.clone();
            let url_entry = url_entry.clone();
            let file_entry = file_entry.clone();
            let paste_buffer = paste_buffer.clone();
            move || {
                let (start, end) = paste_buffer.bounds();
                let pasted = paste_buffer.text(&start, &end, false).to_string();
                let error = subscription_form_error(
                    &name_entry.text(),
                    &url_entry.text(),
                    &file_entry.text(),
                    &pasted,
                );
                match error {
                    Some(message) => {
                        error_label.set_text(&message);
                        error_label.set_visible(true);
                        dialog.set_response_enabled("add", false);
                    }
                    None => {
                        error_label.set_visible(false);
                        dialog.set_response_enabled("add", true);
                    }
                }
            }
        };
        {
            let update_validity = update_validity.clone();
            name_entry.connect_changed(move |_| update_validity());
        }
        {
            let update_validity = update_validity.clone();
            url_entry.connect_changed(move |_| update_validity());
        }
        {
            let update_validity = update_validity.clone();
            file_entry.connect_changed(move |_| update_validity());
        }
        {
            let update_validity = update_validity.clone();
            paste_buffer.connect_changed(move |_| update_validity());
        }
        update_validity();
    }

    dialog.connect_response(None, move |_, response| {
        if response == "add" {
            let name = name_entry.text().to_string();
            let url = strip_v2raytun_import_prefix(url_entry.text().trim());
            let file_path = file_entry.text().to_string();
            let (start, end) = paste_buffer.bounds();
            let pasted_json = paste_buffer.text(&start, &end, false).to_string();
            if !name.trim().is_empty()
                && let Some(source) =
                    subscription_source_from_inputs(&url, file_path.trim(), pasted_json.trim())
            {
                sender.input(SubscriptionsMsg::AddSubscription(
                    name.trim().into(),
                    source,
                ));
            }
        }
    });

    dialog.present(crate::active_window().as_ref());
}

/// Strips a `v2raytun://import/` deep-link prefix and recovers the
/// underlying subscription URL, which the app encodes URL-then-base64.
pub(crate) fn strip_v2raytun_import_prefix(input: &str) -> String {
    let Some(rest) = input.strip_prefix(V2RAYTUN_IMPORT_PREFIX) else {
        return input.to_string();
    };
    let decoded = percent_encoding::percent_decode_str(rest)
        .decode_utf8()
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| rest.to_string());
    base64::engine::general_purpose::STANDARD
        .decode(&decoded)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&decoded))
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or(decoded)
}

pub(crate) fn subscription_source_from_inputs(
    url: &str,
    file_path: &str,
    pasted_json: &str,
) -> Option<SubscriptionSourceInput> {
    let url = url.trim();
    let file_path = file_path.trim();
    let pasted_json = pasted_json.trim();
    match (url.is_empty(), file_path.is_empty(), pasted_json.is_empty()) {
        (false, true, true) => Some(SubscriptionSourceInput::Url(url.into())),
        (true, false, true) => Some(SubscriptionSourceInput::File(file_path.into())),
        (true, true, false) => Some(SubscriptionSourceInput::Paste(pasted_json.into())),
        _ => None,
    }
}

pub(super) fn subscription_form_error(
    name: &str,
    url: &str,
    file_path: &str,
    pasted_json: &str,
) -> Option<String> {
    if name.trim().is_empty() {
        return Some("A subscription name is required".to_string());
    }

    let url = strip_v2raytun_import_prefix(url.trim());
    if subscription_source_from_inputs(&url, file_path, pasted_json).is_some() {
        return None;
    }

    if url.trim().is_empty() && file_path.trim().is_empty() && pasted_json.trim().is_empty() {
        Some("Provide a URL, a local file path, or pasted JSON".to_string())
    } else {
        Some("Provide exactly one subscription source".to_string())
    }
}

pub(super) fn write_subscription_blob(
    paths: &AppPaths,
    content: &str,
) -> Result<std::path::PathBuf, PersistenceError> {
    paths.ensure_dirs()?;
    let path = paths
        .subscription_blobs_dir()
        .join(format!("{}.json", Uuid::new_v4()));
    v2ray_rs_core::fs::atomic_write(&path, content.as_bytes()).map_err(PersistenceError::Io)?;
    Ok(path)
}

pub(super) fn show_rename_dialog(
    id: Uuid,
    current_name: &str,
    sender: ComponentSender<SubscriptionsPage>,
) {
    let dialog = adw::AlertDialog::builder()
        .heading("Rename Subscription")
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("rename", "Rename");
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let name_entry = adw::EntryRow::builder()
        .title("Name")
        .text(current_name)
        .build();

    let group = adw::PreferencesGroup::new();
    group.add(&name_entry);
    content.append(&group);

    dialog.set_extra_child(Some(&content));

    {
        let dialog = dialog.clone();
        let name_entry = name_entry.clone();
        let update_validity = {
            let dialog = dialog.clone();
            let name_entry = name_entry.clone();
            move || {
                dialog.set_response_enabled("rename", !name_entry.text().trim().is_empty());
            }
        };
        {
            let update_validity = update_validity.clone();
            name_entry.connect_changed(move |_| update_validity());
        }
        update_validity();
    }

    dialog.connect_response(None, move |_, response| {
        if response == "rename" {
            let new_name = name_entry.text().to_string();
            if !new_name.trim().is_empty() {
                sender.input(SubscriptionsMsg::RenameSubscription(
                    id,
                    new_name.trim().into(),
                ));
            }
        }
    });

    dialog.present(crate::active_window().as_ref());
}

pub(super) fn show_delete_dialog(id: Uuid, sender: ComponentSender<SubscriptionsPage>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Delete Subscription")
        .body("Are you sure you want to delete this subscription?")
        .build();

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            sender.input(SubscriptionsMsg::DeleteSubscription(id));
        }
    });

    dialog.present(crate::active_window().as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use v2ray_rs_core::profile::AppProfile;

    #[test]
    fn write_subscription_blob_writes_owner_only_mode_0600() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        let path = write_subscription_blob(&paths, r#"{"outbounds":[]}"#).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"outbounds":[]}"#
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
