use adw::prelude::*;
use ipnet::IpNet;
use relm4::adw;
use relm4::gtk;
use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;
use uuid::Uuid;

use v2ray_rs_core::geodata_index::GeodataIndexManager;
use v2ray_rs_core::models::{
    AppSettings, Preset, RoutingRule, RoutingRuleSet, RuleAction, RuleMatch, builtin_presets,
    validate_rule_match,
};
use v2ray_rs_core::persistence::{self, AppPaths};

use super::{RoutingCallback, emit_routing};

pub(super) fn build_routing_page(
    paths: &AppPaths,
    settings_state: &Rc<RefCell<AppSettings>>,
    rule_set: &Rc<RefCell<RoutingRuleSet>>,
    on_changed: &RoutingCallback,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Routing")
        .icon_name("network-workgroup-symbolic")
        .build();

    let paths = Rc::new(paths.clone());

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
        settings_state: settings_state.clone(),
        rule_set: rule_set.clone(),
        on_changed: on_changed.clone(),
        paths: paths.clone(),
        added_groups: Rc::new(RefCell::new(Vec::new())),
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

pub(super) fn build_routing_error_page(paths: &AppPaths, error: String) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Routing")
        .icon_name("network-workgroup-symbolic")
        .build();

    let group = adw::PreferencesGroup::builder()
        .title("Routing Rules Need Repair")
        .description("The stored routing rules could not be loaded")
        .build();

    let row = adw::ActionRow::builder()
        .title("Load failed")
        .subtitle(error)
        .build();

    let status = gtk::Label::builder()
        .label("Reset the broken routing rules file to continue editing.")
        .xalign(1.0)
        .build();
    row.add_suffix(&status);

    let reset_btn = gtk::Button::builder()
        .label("Reset Data")
        .css_classes(["destructive-action"])
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&reset_btn);

    let routing_path = paths.routing_rules_path();
    reset_btn.connect_clicked(move |button| match std::fs::remove_file(&routing_path) {
        Ok(()) => {
            status.set_label("Routing rules reset. Reopen Preferences to continue.");
            button.set_sensitive(false);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            status.set_label("Routing rules reset. Reopen Preferences to continue.");
            button.set_sensitive(false);
        }
        Err(err) => {
            status.set_label(&format!("Reset failed: {err}"));
        }
    });

    group.add(&row);
    page.add(&group);
    page
}

#[derive(Clone)]
struct RenderCtx {
    page: adw::PreferencesPage,
    settings_state: Rc<RefCell<AppSettings>>,
    rule_set: Rc<RefCell<RoutingRuleSet>>,
    on_changed: RoutingCallback,
    paths: Rc<AppPaths>,
    added_groups: Rc<RefCell<Vec<adw::PreferencesGroup>>>,
}

fn emit_routing_change(ctx: &RenderCtx) {
    emit_routing(&ctx.rule_set, &ctx.on_changed);
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
                }
                emit_routing_change(&ctx);
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
            drop(rs);
            emit_routing_change(&ctx);
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
            emit_routing_change(&ctx);
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
            emit_routing_change(&ctx);
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
            emit_routing_change(&ctx);
            render_routing_rules(&ctx);
        });
    }
    popover_box.append(&delete_btn);

    popover.set_child(Some(&popover_box));
    menu_btn.set_popover(Some(&popover));
    row.add_suffix(&menu_btn);

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
            let Some(drop_idx) = value.get::<u32>().ok().map(|v| v as usize) else {
                return false;
            };

            if drop_idx == drop_idx_target {
                return false;
            }

            ctx_drop
                .rule_set
                .borrow_mut()
                .move_rule(drop_idx, drop_idx_target);

            emit_routing_change(&ctx_drop);
            render_routing_rules(&ctx_drop);

            if let Some(w) = target.widget() {
                w.remove_css_class("drop-target");
            }
            true
        },
    );

    drop_target.connect_enter(|target, _, _| {
        if let Some(w) = target.widget() {
            w.add_css_class("drop-target");
        }
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

    let error_label = gtk::Label::builder()
        .css_classes(["error"])
        .halign(gtk::Align::Start)
        .wrap(true)
        .visible(false)
        .build();
    content.append(&error_label);

    dialog.set_extra_child(Some(&content));

    let type_combo_clone = type_combo.clone();
    let value_entry_clone = value_entry.clone();
    let paths = ctx.paths.clone();
    let settings_state = ctx.settings_state.clone();

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

        show_suggestions.connect_clicked(move |_| {
            let rule_type = type_combo.selected();
            let backend_type = settings_state.borrow().backend.backend_type;

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
                                .title(format!("... and {} more", filtered.len() - 20))
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

            (*suggestion_dialog).present(crate::active_window().as_ref());
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
                Err(e) => {
                    value_entry.add_css_class("error");
                    error_label.set_text(&format!("Invalid IP CIDR: {e}"));
                    error_label.set_visible(true);
                    return;
                }
            },
            _ => return,
        };

        if let Err(e) = validate_rule_match(&match_condition) {
            value_entry.add_css_class("error");
            error_label.set_text(&e.to_string());
            error_label.set_visible(true);
            log::warn!("invalid rule match: {e}");
            return;
        }
        value_entry.remove_css_class("error");
        error_label.set_visible(false);

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
        }
        emit_routing_change(&ctx);
        render_routing_rules(&ctx);
    });

    dialog.present(crate::active_window().as_ref());
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
            emit_routing_change(&ctx);
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
                emit_routing_change(&ctx);
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
            let row_ref = row.clone();
            let custom_group_ref = custom_group.clone();
            delete_btn.connect_clicked(move |_| {
                if let Err(e) = persistence::delete_preset(&pp, &name) {
                    log::error!("delete preset: {e}");
                } else {
                    custom_group_ref.remove(&row_ref);
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
    dialog.present(crate::active_window().as_ref());
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

    dialog.present(crate::active_window().as_ref());
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
