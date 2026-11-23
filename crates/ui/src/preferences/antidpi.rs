use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use relm4::{adw, gtk};

use v2ray_rs_core::models::{
    AntiDpiConfig, AppSettings, MAX_ANTIDPI_STRATEGIES, MAX_ANTIDPI_STRATEGY_LEN,
};

use crate::i18n::tr;

use super::{SettingsCallback, SettingsObservers, ToastCallback, subscribe_settings};

pub(crate) const ANTIDPI_PAGE_NAME: &str = "antidpi";

/// The anti-DPI page's editable widgets plus their rollback guards, kept so
/// the settings observer can re-render committed values without re-entering
/// the rows' own change handlers.
pub(super) struct AntidpiRows {
    pub(crate) enable_row: adw::SwitchRow,
    pub(crate) queue_row: adw::SpinRow,
    pub(crate) hostlist_row: adw::EntryRow,
    pub(crate) strategies_buffer: gtk::TextBuffer,
    strategies_scroll: gtk::ScrolledWindow,
    enable_guard: Rc<Cell<bool>>,
    queue_guard: Rc<Cell<bool>>,
    render_guard: Rc<Cell<bool>>,
}

impl AntidpiRows {
    /// Renders the committed anti-DPI settings into the rows. Every widget
    /// write runs under its rollback guard, and validation error styling is
    /// cleared, so a reconciliation shows one clean committed state.
    pub(crate) fn render(&self, antidpi: &AntiDpiConfig) {
        with_rollback_guard(&self.enable_guard, || {
            self.enable_row.set_active(antidpi.enabled);
        });
        with_rollback_guard(&self.queue_guard, || {
            self.queue_row.set_value(f64::from(antidpi.queue));
        });
        self.hostlist_row.remove_css_class("error");
        self.hostlist_row
            .set_text(antidpi.hostlist_path.as_deref().unwrap_or(""));
        self.strategies_scroll.remove_css_class("error");
        with_rollback_guard(&self.render_guard, || {
            self.strategies_buffer
                .set_text(&text_from_strategies(&antidpi.strategies));
        });
    }
}

pub(super) fn build_antidpi_page(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    observers: &SettingsObservers,
    toast_cb: &ToastCallback,
) -> (adw::PreferencesPage, Rc<AntidpiRows>) {
    let page = adw::PreferencesPage::builder()
        .title(tr("Anti-DPI"))
        .icon_name("network-wireless-encrypted-symbolic")
        .build();
    page.set_name(Some(ANTIDPI_PAGE_NAME));

    // --- Primary group ------------------------------------------------------
    let primary = adw::PreferencesGroup::builder()
        .title(tr("Anti-DPI engine"))
        .description(tr(
            "Inspects the early TLS and QUIC packets of flows to port 443 on host egress; works with every backend, with or without TUN. The optional hostlist matches TLS SNI or HTTP Host, never QUIC. There is no guarantee that the first packet is covered, and IPv6 traffic passes through unprocessed",
        ))
        .build();

    let enable_row = adw::SwitchRow::builder()
        .title(tr("Enable anti-DPI"))
        .subtitle(tr("Applies on next connect"))
        .active(state.borrow().antidpi.enabled)
        .build();
    primary.add(&enable_row);

    let queue_row = adw::SpinRow::builder()
        .title(tr("NFQUEUE number"))
        .subtitle(tr(
            "Must differ from any other NFQUEUE consumer on this machine",
        ))
        .adjustment(&gtk::Adjustment::new(
            f64::from(state.borrow().antidpi.queue),
            1.0,
            65535.0,
            1.0,
            10.0,
            0.0,
        ))
        .build();
    primary.add(&queue_row);

    let hostlist_row = adw::EntryRow::builder()
        .title(tr("Hostlist file"))
        .text(
            state
                .borrow()
                .antidpi
                .hostlist_path
                .as_deref()
                .unwrap_or(""),
        )
        .show_apply_button(true)
        .build();
    primary.add(&hostlist_row);
    page.add(&primary);

    // --- Strategies ----------------------------------------------------------
    let strategies_group = adw::PreferencesGroup::builder()
        .title(tr("Strategy chains"))
        .description(tr(
            "One strategy chain per line; an empty list uses the helper's built-in defaults. Applies on next connect",
        ))
        .build();

    let strategies_buffer = gtk::TextBuffer::new(None);
    strategies_buffer.set_text(&text_from_strategies(&state.borrow().antidpi.strategies));
    let strategies_view = gtk::TextView::builder()
        .buffer(&strategies_buffer)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(6)
        .right_margin(6)
        .build();
    let strategies_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(120)
        .max_content_height(320)
        .hexpand(true)
        .child(&strategies_view)
        .build();
    strategies_group.add(&strategies_scroll);
    page.add(&strategies_group);

    // --- Privileges hint ------------------------------------------------------
    let cap_group = adw::PreferencesGroup::builder()
        .title(tr("Privileges"))
        .build();
    let cap_hint = adw::ActionRow::builder()
        .title(tr("Capability status"))
        .subtitle(tr(
            "The engine needs cap_net_admin and cap_net_raw; the Grant TUN privileges action on the TUN page covers the anti-DPI helper in the same elevation",
        ))
        .sensitive(false)
        .build();
    cap_group.add(&cap_hint);
    page.add(&cap_group);

    // --- Handlers -----------------------------------------------------------
    let enable_guard = Rc::new(Cell::new(false));
    let queue_guard = Rc::new(Cell::new(false));
    let render_guard = Rc::new(Cell::new(false));
    let rows = Rc::new(AntidpiRows {
        enable_row: enable_row.clone(),
        queue_row: queue_row.clone(),
        hostlist_row: hostlist_row.clone(),
        strategies_buffer: strategies_buffer.clone(),
        strategies_scroll: strategies_scroll.clone(),
        enable_guard: enable_guard.clone(),
        queue_guard: queue_guard.clone(),
        render_guard: render_guard.clone(),
    });
    {
        let state = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        let guard = enable_guard.clone();
        enable_row.connect_active_notify(move |row| {
            if guard.get() {
                return;
            }
            let enabled = row.is_active();
            if apply_antidpi_mutation(&state, &cb, |s| {
                s.antidpi.enabled = enabled;
                Ok(())
            })
            .is_err()
            {
                toast_cb(&tr("Failed to save anti-DPI setting"));
                with_rollback_guard(&guard, || {
                    row.set_active(state.borrow().antidpi.enabled);
                });
                return;
            }
            if enabled {
                toast_cb(&tr(
                    "The anti-DPI engine inspects outgoing traffic; applies on next connect.",
                ));
            }
        });
    }
    {
        let state = state.clone();
        let toast_cb = toast_cb.clone();
        let guard = queue_guard.clone();
        // Validate the committed decimal text via the `input` signal, not
        // per keystroke: intermediate text and the temporarily empty field
        // during replacement are neither rejected nor applied. The accepted
        // value is committed by the `notify::value` handler below, which
        // also covers the spin buttons: they change the value without ever
        // emitting `input`.
        queue_row.connect_input(move |row| {
            if guard.get() {
                return None;
            }
            let Some(queue) = queue_from_text(&row.text()) else {
                toast_cb(&tr(
                    "The NFQUEUE number must be a whole number from 1 to 65535",
                ));
                return Some(Ok(f64::from(state.borrow().antidpi.queue)));
            };
            Some(Ok(f64::from(queue)))
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        let guard = queue_guard.clone();
        queue_row.connect_value_notify(move |row| {
            if guard.get() {
                return;
            }
            let value = row.value();
            if value.fract() != 0.0 || !(1.0..=f64::from(u16::MAX)).contains(&value) {
                toast_cb(&tr(
                    "The NFQUEUE number must be a whole number from 1 to 65535",
                ));
                with_rollback_guard(&guard, || {
                    row.set_value(f64::from(state.borrow().antidpi.queue));
                });
                return;
            }
            let queue = value as u16;
            if apply_antidpi_mutation(&state, &cb, |s| {
                if let Some(message) = queue_field_error(queue) {
                    return Err(message);
                }
                s.antidpi.queue = queue;
                Ok(())
            })
            .is_err()
            {
                toast_cb(&tr("Failed to save anti-DPI setting"));
            }
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        hostlist_row.connect_apply(move |row| {
            // Exactly empty clears the path; whitespace-only input is
            // rejected; anything else is stored verbatim.
            let parsed = match hostlist_path_from_text(&row.text()) {
                Ok(parsed) => parsed,
                Err(()) => {
                    toast_cb(&tr("The hostlist path must not be blank"));
                    row.add_css_class("error");
                    return;
                }
            };
            if apply_antidpi_mutation(&state, &cb, |s| {
                if let Some(message) = hostlist_field_error(parsed.as_deref()) {
                    return Err(message);
                }
                s.antidpi.hostlist_path = parsed.clone();
                Ok(())
            })
            .is_err()
            {
                toast_cb(&tr("Failed to save anti-DPI setting"));
                row.add_css_class("error");
                row.set_text(
                    state
                        .borrow()
                        .antidpi
                        .hostlist_path
                        .as_deref()
                        .unwrap_or(""),
                );
                return;
            }
            row.remove_css_class("error");
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        let buffer = strategies_buffer.clone();
        let scroll = strategies_scroll.clone();
        // Commit only when editing finishes: a per-keystroke commit would
        // fight the user with validation rollbacks on half-typed chains.
        let focus = gtk::EventControllerFocus::new();
        let render = render_guard.clone();
        focus.connect_leave(move |_| {
            if render.get() {
                return;
            }
            let (start, end) = buffer.bounds();
            let text = buffer.text(&start, &end, true);
            // The whole edit is applied or none of it: every field failure
            // maps to one localized message and the saved list is restored.
            match commit_strategies(&state, &cb, &text) {
                Ok(()) => scroll.remove_css_class("error"),
                Err(message) => {
                    toast_cb(&message);
                    scroll.add_css_class("error");
                    // Restore the buffer to the saved state; `render` keeps the
                    // restore from re-entering this handler.
                    render.set(true);
                    let saved = state.borrow().antidpi.strategies.clone();
                    buffer.set_text(&text_from_strategies(&saved));
                    render.set(false);
                }
            }
        });
        strategies_view.add_controller(focus);
    }

    // The rows observe committed settings so a failed disk write can be
    // reconciled back to the persisted snapshot: the observer renders under
    // the rollback guards and never re-enters the commit path.
    let observer_rows = rows.clone();
    subscribe_settings(observers, move |settings| {
        observer_rows.render(&settings.antidpi);
    });

    (page, rows)
}

/// Parses the strategy editor's text into chains. Only the exactly empty
/// editor clears the list, and exactly one trailing line terminator is
/// permitted; whitespace-only non-empty content rejects the edit. Blank or
/// whitespace-only lines reject the edit, while remaining lines are kept
/// verbatim.
fn strategies_from_text(text: &str) -> Result<Vec<String>, ()> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    if lines.is_empty() || lines == [""] {
        return Ok(Vec::new());
    }
    lines
        .into_iter()
        .map(|line| {
            if line.trim().is_empty() {
                Err(())
            } else {
                Ok(line.to_string())
            }
        })
        .collect()
}

/// Renders the saved chains back into the editor: one per line.
fn text_from_strategies(strategies: &[String]) -> String {
    strategies.join("\n")
}

/// Validates and commits the strategy editor's complete text: parsing, the
/// count cap, the per-chain character cap, then the stored-config check. A
/// rejected edit leaves the saved list untouched and returns its one
/// localized message.
fn commit_strategies(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    text: &str,
) -> Result<(), String> {
    let strategies = strategies_from_text(text)
        .map_err(|()| tr("Strategy chains must not contain blank lines"))?;
    if let Some(message) = strategies_field_error(&strategies) {
        return Err(message);
    }
    apply_antidpi_mutation(state, cb, |s| {
        s.antidpi.strategies = strategies;
        Ok(())
    })
    .map_err(|_| tr("Failed to save anti-DPI strategies"))
}

/// Field validators against the current settings, applied independently so
/// one rejected field cannot block a correction to another field of a
/// hand-edited section.
fn queue_field_error(queue: u16) -> Option<String> {
    (queue == 0).then(|| tr("The NFQUEUE number must be a whole number from 1 to 65535"))
}

fn hostlist_field_error(path: Option<&str>) -> Option<String> {
    path.filter(|path| path.trim().is_empty())
        .map(|_| tr("The hostlist path must not be blank"))
}

fn strategies_field_error(strategies: &[String]) -> Option<String> {
    if strategies.len() > MAX_ANTIDPI_STRATEGIES {
        return Some(tr("Too many strategy chains; the edit was not saved"));
    }
    if strategies
        .iter()
        .any(|chain| chain.chars().count() > MAX_ANTIDPI_STRATEGY_LEN)
    {
        return Some(tr("Each strategy chain is limited to 512 characters"));
    }
    None
}

/// Parses the spin row's committed decimal text before the adjustment sees
/// it: empty, non-numeric, zero, and out-of-range input all fail, so a
/// rejected edit never silently clamps to a different queue.
fn queue_from_text(text: &str) -> Option<u16> {
    let queue = text.trim().parse::<u16>().ok()?;
    (queue != 0).then_some(queue)
}

/// Classifies the hostlist field's committed text: exactly empty clears the
/// saved path, whitespace-only input is rejected, and anything else is kept
/// verbatim.
fn hostlist_path_from_text(text: &str) -> Result<Option<String>, ()> {
    if text.is_empty() {
        return Ok(None);
    }
    if text.trim().is_empty() {
        return Err(());
    }
    Ok(Some(text.to_string()))
}

/// Restores a widget after a rejected mutation without letting the restore
/// re-enter its own change handler. Mirrors the TUN page's guard.
fn with_rollback_guard(guard: &Cell<bool>, rollback: impl FnOnce()) {
    if guard.replace(true) {
        return;
    }
    rollback();
    guard.set(false);
}

/// Clone → mutate → validate → commit → emit, the `apply_tun_mutation`
/// pattern; the mutation validates its own edited field and returns its
/// localized error, so a rejected mutation leaves the stored settings
/// untouched and other fields of a hand-edited section stay repairable.
fn apply_antidpi_mutation<F>(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    mutate: F,
) -> Result<(), String>
where
    F: FnOnce(&mut AppSettings) -> Result<(), String>,
{
    let mut next = state.borrow().clone();
    mutate(&mut next)?;
    *state.borrow_mut() = next.clone();
    cb(next);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategies_parse_keeps_lines_verbatim_and_allows_one_final_terminator() {
        assert_eq!(
            strategies_from_text("fake:blob=tls:sni=a.example.com\n  multisplit:pos=midsld  "),
            Ok(vec![
                "fake:blob=tls:sni=a.example.com".to_string(),
                "  multisplit:pos=midsld  ".to_string(),
            ])
        );
        assert_eq!(
            strategies_from_text("multisplit:pos=midsld\n"),
            Ok(vec!["multisplit:pos=midsld".to_string()])
        );
    }

    #[test]
    fn strategies_blank_interior_lines_reject_the_edit() {
        assert!(strategies_from_text("multisplit:pos=midsld\n\nfake:blob=tls").is_err());
        assert!(strategies_from_text("multisplit:pos=midsld\n   \nfake:blob=tls").is_err());
        assert!(strategies_from_text("\nmultisplit:pos=midsld").is_err());
        assert!(strategies_from_text("multisplit:pos=midsld\n\n").is_err());
        assert!(strategies_from_text("\n\n").is_err());
    }

    #[test]
    fn only_an_exactly_empty_editor_clears_the_strategy_list() {
        assert_eq!(strategies_from_text(""), Ok(Vec::new()));
        assert_eq!(strategies_from_text("\n"), Ok(Vec::new()));
        assert!(strategies_from_text("   ").is_err());
        assert!(strategies_from_text(" \n").is_err());
        assert!(strategies_from_text("  \n \n").is_err());
    }

    #[test]
    fn strategies_render_one_per_line() {
        let text = text_from_strategies(&[
            "multisplit:pos=midsld".to_string(),
            "multidisorder:pos=midsld".to_string(),
        ]);
        assert_eq!(text, "multisplit:pos=midsld\nmultidisorder:pos=midsld");
        assert_eq!(text_from_strategies(&[]), "");
    }

    #[test]
    fn strategies_round_trip_through_the_editor_text() {
        let chains = vec![
            "fake:blob=tls:tls_mod=rnd,dupsid,sni=fonts.google.com:tcp_seq=10000".to_string(),
            "multisplit:pos=2,midsld:seqovl=1".to_string(),
        ];
        assert_eq!(
            strategies_from_text(&text_from_strategies(&chains)).unwrap(),
            chains
        );
    }

    #[test]
    fn apply_antidpi_mutation_commits_valid_changes_and_emits() {
        let state = Rc::new(RefCell::new(AppSettings::default()));
        let emitted = Rc::new(RefCell::new(0_u32));
        let emitted_sink = emitted.clone();
        let cb: SettingsCallback = Rc::new(move |_| {
            *emitted_sink.borrow_mut() += 1;
        });
        apply_antidpi_mutation(&state, &cb, |s| {
            s.antidpi.enabled = true;
            s.antidpi.queue = 9;
            Ok(())
        })
        .unwrap();
        let settings = state.borrow();
        assert!(settings.antidpi.enabled);
        assert_eq!(settings.antidpi.queue, 9);
        assert_eq!(*emitted.borrow(), 1);
    }

    #[test]
    fn per_field_validators_reject_only_their_field() {
        assert_eq!(
            queue_field_error(0),
            Some(tr(
                "The NFQUEUE number must be a whole number from 1 to 65535"
            ))
        );
        assert_eq!(queue_field_error(200), None);
        assert_eq!(
            hostlist_field_error(Some("   ")),
            Some(tr("The hostlist path must not be blank"))
        );
        assert_eq!(
            hostlist_field_error(Some("/etc/v2ray-rs/hosts.txt")),
            None,
            "a non-blank path is accepted verbatim"
        );
        assert_eq!(hostlist_field_error(None), None);

        let oversized = "x".repeat(513);
        assert_eq!(
            strategies_field_error(&[oversized]),
            Some(tr("Each strategy chain is limited to 512 characters"))
        );
        assert_eq!(
            strategies_field_error(&["multisplit:pos=midsld".to_string()]),
            None
        );
    }

    #[test]
    fn a_valid_field_edit_persists_while_another_field_stays_invalid() {
        let state = Rc::new(RefCell::new(AppSettings::default()));
        // A hand-edited section carries an invalid queue and a blank
        // hostlist; fixing the strategies must not be blocked by them.
        state.borrow_mut().antidpi.queue = 0;
        state.borrow_mut().antidpi.hostlist_path = Some("   ".to_string());
        state.borrow_mut().antidpi.strategies = vec!["multisplit:pos=midsld".to_string()];
        let emitted = Rc::new(RefCell::new(0_u32));
        let emitted_sink = emitted.clone();
        let cb: SettingsCallback = Rc::new(move |_| {
            *emitted_sink.borrow_mut() += 1;
        });

        commit_strategies(
            &state,
            &cb,
            "multisplit:pos=midsld\nmultidisorder:pos=midsld",
        )
        .expect("a valid strategies edit must persist despite other invalid fields");

        {
            let settings = state.borrow();
            assert_eq!(settings.antidpi.strategies.len(), 2);
            assert_eq!(settings.antidpi.queue, 0, "invalid queue left untouched");
        }
        assert_eq!(*emitted.borrow(), 1);

        // The rejected queue field reports only its own error and keeps the
        // other committed fields.
        let err = apply_antidpi_mutation(&state, &cb, |s| {
            s.antidpi.queue = 0;
            if let Some(message) = queue_field_error(s.antidpi.queue) {
                return Err(message);
            }
            Ok(())
        })
        .expect_err("queue 0 must be rejected");
        assert_eq!(
            err,
            tr("The NFQUEUE number must be a whole number from 1 to 65535")
        );
        assert_eq!(state.borrow().antidpi.strategies.len(), 2);
        assert_eq!(*emitted.borrow(), 1, "rejected edit must not emit");
    }

    #[test]
    fn over_limit_edit_is_rejected_in_full_with_the_frozen_notice() {
        let state = Rc::new(RefCell::new(AppSettings::default()));
        let saved = vec!["multisplit:pos=midsld".to_string()];
        state.borrow_mut().antidpi.strategies = saved.clone();
        let emitted = Rc::new(RefCell::new(0_u32));
        let emitted_sink = emitted.clone();
        let cb: SettingsCallback = Rc::new(move |_| {
            *emitted_sink.borrow_mut() += 1;
        });

        // One more chain than the limit, each chain individually valid, so
        // the count is the only reason the edit fails.
        let chains: Vec<String> = (0..=MAX_ANTIDPI_STRATEGIES)
            .map(|i| format!("fake:blob=tls:sni=h{i}.example.com"))
            .collect();
        assert_eq!(chains.len(), MAX_ANTIDPI_STRATEGIES + 1);

        let err = commit_strategies(&state, &cb, &text_from_strategies(&chains))
            .expect_err("an over-limit edit must be rejected in full");

        assert_eq!(
            state.borrow().antidpi.strategies,
            saved,
            "rejected edit must leave the saved list unchanged"
        );
        assert_eq!(*emitted.borrow(), 0, "rejected edit must not emit");
        assert_eq!(err, tr("Too many strategy chains; the edit was not saved"));
    }

    #[test]
    fn oversized_chain_is_rejected_with_a_localized_message() {
        let state = Rc::new(RefCell::new(AppSettings::default()));
        let cb: SettingsCallback = Rc::new(|_| {});

        let text = "x".repeat(MAX_ANTIDPI_STRATEGY_LEN + 1);
        let err =
            commit_strategies(&state, &cb, &text).expect_err("an oversized chain must be rejected");

        assert_eq!(err, tr("Each strategy chain is limited to 512 characters"));
        assert!(
            !err.contains("strategy chain longer than"),
            "the English validator detail must not leak into the UI message: {err}"
        );
        assert_eq!(state.borrow().antidpi.strategies, Vec::<String>::new());
    }

    #[test]
    fn blank_interior_line_rejects_the_whole_strategy_edit() {
        let state = Rc::new(RefCell::new(AppSettings::default()));
        let saved = vec!["multisplit:pos=midsld".to_string()];
        state.borrow_mut().antidpi.strategies = saved.clone();
        let emitted = Rc::new(RefCell::new(0_u32));
        let emitted_sink = emitted.clone();
        let cb: SettingsCallback = Rc::new(move |_| {
            *emitted_sink.borrow_mut() += 1;
        });

        let err = commit_strategies(&state, &cb, "multisplit:pos=2\n\nmultisplit:pos=midsld")
            .expect_err("a blank interior line must reject the edit");

        assert_eq!(err, tr("Strategy chains must not contain blank lines"));
        assert_eq!(state.borrow().antidpi.strategies, saved);
        assert_eq!(*emitted.borrow(), 0);
    }

    #[test]
    fn queue_from_text_rejects_empty_nonnumeric_zero_and_out_of_range_input() {
        assert_eq!(queue_from_text(""), None);
        assert_eq!(queue_from_text("   "), None);
        assert_eq!(queue_from_text("abc"), None);
        assert_eq!(queue_from_text("2.5"), None);
        assert_eq!(queue_from_text("-1"), None);
        assert_eq!(queue_from_text("0"), None);
        assert_eq!(queue_from_text("000"), None);
        assert_eq!(queue_from_text("65536"), None);
        assert_eq!(queue_from_text("1"), Some(1));
        assert_eq!(queue_from_text("65535"), Some(65535));
        assert_eq!(queue_from_text(" 200 "), Some(200));
    }

    #[test]
    fn hostlist_only_an_exactly_empty_field_clears_the_path() {
        assert_eq!(hostlist_path_from_text(""), Ok(None));
        assert!(hostlist_path_from_text(" ").is_err());
        assert!(hostlist_path_from_text("\t").is_err());
        assert!(hostlist_path_from_text("   ").is_err());
        assert_eq!(
            hostlist_path_from_text("/etc/v2ray-rs/hosts.txt"),
            Ok(Some("/etc/v2ray-rs/hosts.txt".to_string()))
        );
        // Any other input is kept verbatim, whitespace included.
        assert_eq!(
            hostlist_path_from_text(" /etc/hosts.txt "),
            Ok(Some(" /etc/hosts.txt ".to_string()))
        );
    }

    #[test]
    fn empty_editor_saves_an_empty_strategy_list() {
        let state = Rc::new(RefCell::new(AppSettings::default()));
        state.borrow_mut().antidpi.strategies = vec!["multisplit:pos=midsld".to_string()];
        let emitted = Rc::new(RefCell::new(0_u32));
        let emitted_sink = emitted.clone();
        let cb: SettingsCallback = Rc::new(move |_| {
            *emitted_sink.borrow_mut() += 1;
        });

        // The exactly empty editor parses to an empty list, which is valid:
        // it delegates to the helper's built-in defaults.
        commit_strategies(&state, &cb, "").expect("an empty editor clears the list");

        assert!(state.borrow().antidpi.strategies.is_empty());
        assert_eq!(*emitted.borrow(), 1);
    }

    #[test]
    fn rejected_write_reconciles_rows_to_persisted_settings() {
        crate::gtk_test::run(|| {
            let state = Rc::new(RefCell::new(AppSettings::default()));
            let observers: SettingsObservers = Rc::new(RefCell::new(Vec::new()));
            // The dialog's commit callback only reaches the app, where the
            // disk write fails; it never re-enters the dialog, so it stays
            // silent here. The stored state still holds the unapplied edit.
            let emitted = Rc::new(RefCell::new(0_u32));
            let emitted_sink = emitted.clone();
            let cb: SettingsCallback = Rc::new(move |_| {
                *emitted_sink.borrow_mut() += 1;
            });
            let toast_cb: ToastCallback = Rc::new(|_| {});

            let (page, rows) = build_antidpi_page(&state, &cb, &observers, &toast_cb);
            let _ = &page;
            assert_eq!(observers.borrow().len(), 1);

            // The dialog accepted the edit locally: the rows and the dialog
            // state show values the app never persisted.
            rows.enable_row.set_active(true);
            rows.queue_row.set_value(9.0);
            assert!(state.borrow().antidpi.enabled);
            assert_eq!(state.borrow().antidpi.queue, 9);
            assert_eq!(*emitted.borrow(), 2);

            // The app still holds the last committed snapshot: that is what
            // the restart banner compares against and what a reconnect uses,
            // so reconciling to it keeps both consistent with the rows.
            let committed = AppSettings::default();
            super::super::reconcile_settings(&state, &observers, &committed);

            assert_eq!(state.borrow().antidpi, committed.antidpi);
            assert!(!rows.enable_row.is_active());
            assert_eq!(rows.queue_row.value(), 200.0);
            assert_eq!(rows.hostlist_row.text().as_str(), "");
            let (start, end) = rows.strategies_buffer.bounds();
            assert_eq!(rows.strategies_buffer.text(&start, &end, true).as_str(), "");
            // Reconciliation must not re-emit: one save error was already
            // reported for the failed write, and no second save is scheduled.
            assert_eq!(*emitted.borrow(), 2);
        });
    }
}
