use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use relm4::gtk::glib;
use relm4::{adw, gtk};

use v2ray_rs_core::models::{
    AppSettings, BackendType, DnsHijackMode, TunStack, validate_domain_pattern,
    validate_exclude_route, validate_ip_cidr, validate_tun_interface_name,
};

use crate::i18n::tr;

use super::{SettingsCallback, SettingsObservers, ToastCallback, subscribe_settings};

type RenderFn = Rc<dyn Fn()>;

pub(crate) const TUN_PAGE_NAME: &str = "tun";

/// One blocking snapshot of the route-helper and anti-DPI-helper facts.
/// Everything here hits the filesystem or NSS, so it must be collected off
/// the GTK thread.
#[derive(Clone)]
struct HelperProbe {
    helper: PathBuf,
    access: v2ray_rs_process::HelperAccess,
    /// `has_net_admin` on the helper, probed only when access is `Ready`.
    capability: Option<Result<bool, String>>,
    /// Whether the relocated helper predates the bundled source.
    stale: bool,
    /// Whether a grant can install the helper by relocating the bundled copy.
    missing_installable: bool,
    /// The anti-DPI helper facts, probed in the same pass: the engine runs
    /// with every backend, so its helper is decided regardless of the TUN
    /// stack, and the shared grant repairs it in the same elevation.
    antidpi: AntiDpiProbe,
}

/// One blocking snapshot of the resolved anti-DPI helper facts.
#[derive(Clone)]
struct AntiDpiProbe {
    path: PathBuf,
    access: v2ray_rs_process::HelperAccess,
    /// `has_antidpi_caps` on the resolved path, probed only when access is
    /// `Ready`; a Ready access already implies an absolute path.
    capability: Option<Result<bool, String>>,
    /// Whether the installed copy predates the bundled source.
    stale: bool,
    /// Whether the grant planner trusts this path as a target: a packaged
    /// or relocated copy it can repair, including installing a missing
    /// helper from the bundled source. An unmanaged copy is package
    /// repair and never a grant target.
    grant_eligible: bool,
}

fn probe_helper() -> HelperProbe {
    let helper = v2ray_rs_process::helper_path();
    let access = v2ray_rs_process::helper_access(&helper);
    let ready = access == v2ray_rs_process::HelperAccess::Ready;
    let capability =
        ready.then(|| v2ray_rs_process::has_net_admin(&helper).map_err(|err| err.to_string()));
    let stale = ready && v2ray_rs_process::helpers_stale(&helper);
    let missing_installable = v2ray_rs_process::relocation_required();
    HelperProbe {
        helper,
        access,
        capability,
        stale,
        missing_installable,
        antidpi: probe_antidpi(),
    }
}

fn probe_antidpi() -> AntiDpiProbe {
    let path = v2ray_rs_process::antidpi_path();
    let access = v2ray_rs_process::helper_access(&path);
    let ready = access == v2ray_rs_process::HelperAccess::Ready;
    let capability =
        ready.then(|| v2ray_rs_process::has_antidpi_caps(&path).map_err(|err| err.to_string()));
    let stale = ready && antidpi_stale(&path);
    // The grant planner owns target eligibility: only a packaged or
    // relocated copy is in the trusted target set, and eligibility also
    // covers installing a missing helper from the bundled source. The
    // trusted set is never expanded from here.
    let grant_eligible = v2ray_rs_process::antidpi_grant_eligible(Some(&path));
    AntiDpiProbe {
        path,
        access,
        capability,
        stale,
        grant_eligible,
    }
}

/// Whether the installed anti-DPI helper predates the bundled one. Mirrors
/// the route helper's metadata-only rule; `helpers_stale` keeps that rule
/// bound to the route helper's source, so the same size/mtime comparison is
/// repeated here against the anti-DPI bundle.
fn antidpi_stale(installed: &Path) -> bool {
    let Some(src) = v2ray_rs_process::bundled_antidpi_path() else {
        return false;
    };
    let (Ok(src_meta), Ok(dst_meta)) = (src.metadata(), installed.metadata()) else {
        return false;
    };
    if src_meta.len() != dst_meta.len() {
        return true;
    }
    match (src_meta.modified().ok(), dst_meta.modified().ok()) {
        (Some(src), Some(dst)) => src > dst,
        // Unreadable timestamps are not evidence of staleness; a same-size
        // helper is far more likely current than not.
        _ => false,
    }
}

/// Maps the probed helper facts to `(message, grant_visible)`, or `None` when
/// the helper is fully ready. The backend holding CAP_NET_ADMIN says nothing
/// about the helper, which is granted separately and can be missing, stale, or
/// simply never have taken the capability -- the package hook tolerates
/// `setcap` failing with a warning.
fn helper_install_reason(probe: &HelperProbe) -> Option<(String, bool)> {
    let message = |access: &v2ray_rs_process::HelperAccess| {
        v2ray_rs_process::ProcessError::from_helper_access(&probe.helper, access.clone())
            .expect("non-Ready access always maps to an error")
            .to_string()
    };
    match &probe.access {
        v2ray_rs_process::HelperAccess::Ready => {
            if probe.stale {
                return Some((
                    "Installed route helper is out of date; grant again".to_string(),
                    true,
                ));
            }
            match &probe.capability {
                Some(Ok(true)) | None => None,
                Some(Ok(false)) => Some(("Route helper lacks CAP_NET_ADMIN".to_string(), true)),
                // Unreadable capabilities are not proof of absence; do not
                // offer a grant that may be redundant.
                Some(Err(err)) => Some((format!("Cannot read capabilities: {err}"), false)),
            }
        }
        v2ray_rs_process::HelperAccess::Missing => {
            if probe.missing_installable {
                Some((
                    "Route helper needs installing before TUN can start".to_string(),
                    true,
                ))
            } else {
                Some((
                    "Route helper is not installed; install the v2ray-rs distribution package"
                        .to_string(),
                    false,
                ))
            }
        }
        access @ (v2ray_rs_process::HelperAccess::EnrollmentRequired
        | v2ray_rs_process::HelperAccess::Permissions) => Some((message(access), true)),
        access @ (v2ray_rs_process::HelperAccess::ReloginRequired
        | v2ray_rs_process::HelperAccess::NoExec
        | v2ray_rs_process::HelperAccess::ProbeFailed(_)) => Some((message(access), false)),
    }
}

/// How the capability row treats the optional anti-DPI helper. The helper is
/// optional: only a terminal verdict on a grant-eligible helper may outrank
/// a fixable backend, and a helper outside the trusted target set never does.
enum AntidpiVerdict {
    Ready,
    /// The grant-eligible helper is repaired by the shared grant.
    Fixable(String),
    /// Reported without a grant because no grant can repair it; it also
    /// outranks a fixable backend.
    Terminal(String),
    /// Outside the trusted target set (absent without a bundled source, or
    /// an unmanaged copy): repair by installing the distribution package,
    /// never by the shared grant.
    PackageRepair(String),
}

fn package_repair_message() -> String {
    tr("Anti-DPI helper is not managed by v2ray-rs; install the distribution package")
}

/// Maps the probed anti-DPI facts to a verdict. Same repair matrix as the
/// route helper for a grant-eligible helper: missing-but-installable, stale,
/// caps-less, unenrolled, and wrong-mode copies are fixed by the shared
/// grant; verification failures and terminal classifications are reported
/// without one. A helper the grant planner does not trust (absent without a
/// bundled source, or an unmanaged copy) is downgraded to package repair so
/// it can neither offer nor block a grant.
fn antidpi_install_reason(probe: &AntiDpiProbe) -> AntidpiVerdict {
    let message = |access: &v2ray_rs_process::HelperAccess| {
        v2ray_rs_process::ProcessError::from_antidpi_access(&probe.path, access.clone())
            .expect("non-Ready access always maps to an error")
            .to_string()
    };
    let classify = |fixable: bool, message: String| {
        if !probe.grant_eligible {
            return AntidpiVerdict::PackageRepair(package_repair_message());
        }
        if fixable {
            AntidpiVerdict::Fixable(message)
        } else {
            AntidpiVerdict::Terminal(message)
        }
    };
    match &probe.access {
        v2ray_rs_process::HelperAccess::Ready => {
            if probe.stale {
                return classify(
                    true,
                    tr("Installed anti-DPI helper is out of date; grant again"),
                );
            }
            match &probe.capability {
                Some(Ok(true)) | None => AntidpiVerdict::Ready,
                Some(Ok(false)) => classify(
                    true,
                    tr("Anti-DPI helper lacks cap_net_admin or cap_net_raw; grant again"),
                ),
                // Unreadable capabilities are not proof of absence; do not
                // offer a grant that may be redundant.
                Some(Err(err)) => classify(
                    false,
                    format!("{}: {err}", tr("Cannot read anti-DPI capabilities")),
                ),
            }
        }
        v2ray_rs_process::HelperAccess::Missing => {
            if probe.grant_eligible {
                AntidpiVerdict::Fixable(tr(
                    "Anti-DPI helper needs installing before the anti-DPI engine can start",
                ))
            } else {
                AntidpiVerdict::PackageRepair(message(&probe.access))
            }
        }
        access @ (v2ray_rs_process::HelperAccess::EnrollmentRequired
        | v2ray_rs_process::HelperAccess::Permissions) => classify(true, message(access)),
        access @ (v2ray_rs_process::HelperAccess::ReloginRequired
        | v2ray_rs_process::HelperAccess::NoExec
        | v2ray_rs_process::HelperAccess::ProbeFailed(_)) => classify(false, message(access)),
    }
}

/// Chooses the capability row's message and Grant visibility from the probed
/// backend, route-helper, and anti-DPI-helper facts. A terminal (no-grant)
/// verdict on the route helper, or on a grant-eligible anti-DPI helper, wins
/// over a fixable backend, so the shared grant button is never offered for a
/// click that cannot repair everything it touches. The optional anti-DPI
/// helper is decided regardless of backend: the engine runs with every
/// backend. A helper outside the trusted target set never suppresses a
/// fixable backend or route grant; its package guidance rides along in the
/// message instead.
fn capability_status(
    backend_caps: Option<&Result<bool, String>>,
    helper_needed: bool,
    probe: &HelperProbe,
) -> (String, bool) {
    let helper_status = helper_needed
        .then(|| helper_install_reason(probe))
        .flatten();
    if let Some((message, false)) = helper_status {
        return (message, false);
    }
    let antidpi_status = antidpi_install_reason(&probe.antidpi);
    if let AntidpiVerdict::Terminal(message) = &antidpi_status {
        return (message.clone(), false);
    }
    let antidpi_repair = match &antidpi_status {
        AntidpiVerdict::Fixable(message) => Some(message.clone()),
        AntidpiVerdict::PackageRepair(message) => Some(message.clone()),
        _ => None,
    };
    match backend_caps {
        None => ("Configure a backend binary first".to_string(), false),
        Some(Ok(true)) => match helper_status {
            Some((message, grant)) => match &antidpi_status {
                // A route or backend grant stays visible; the unmanaged
                // helper's package guidance rides along instead of being
                // discarded.
                AntidpiVerdict::PackageRepair(extra) => (format!("{message}; {extra}"), grant),
                _ => (message, grant),
            },
            None => match antidpi_repair {
                Some(message) => {
                    let grant = matches!(antidpi_status, AntidpiVerdict::Fixable(_));
                    (message, grant)
                }
                None => ("CAP_NET_ADMIN granted".to_string(), false),
            },
        },
        Some(Ok(false)) => match antidpi_repair {
            Some(message) => (
                format!("{}; {}", "Backend lacks CAP_NET_ADMIN", message),
                true,
            ),
            None => ("Backend lacks CAP_NET_ADMIN".to_string(), true),
        },
        // An unreadable verdict is not "definitely missing", so no grant.
        Some(Err(err)) => (format!("Cannot read capabilities: {err}"), false),
    }
}

/// The exact targets a grant offered to repair, captured before elevation.
/// Confirmed success requires these same post-grant destination paths
/// healthy after the re-probe: a helper that disappeared or resolved to a
/// different, unmanaged copy does not count as success.
#[derive(Clone)]
struct GrantTargets {
    backend_path: PathBuf,
    helper_path: PathBuf,
    antidpi_path: PathBuf,
}

impl GrantTargets {
    /// The expected post-grant destinations for a grant planned for the
    /// resolved `helper` and `antidpi` paths, mirroring `plan_grant`: a
    /// relocating plan (AppImage, `nosuid` mount) installs the bundled
    /// copies under `RELOCATE_DIR`, an in-place plan keeps the resolved
    /// paths, and a relocating plan without a bundled anti-DPI source has
    /// no anti-DPI target, so the pre-grant resolution stays the
    /// comparison point.
    fn for_plan(
        backend_path: PathBuf,
        helper: &Path,
        antidpi: &Path,
        relocating: bool,
        antidpi_relocates: bool,
    ) -> GrantTargets {
        GrantTargets {
            backend_path,
            helper_path: if relocating {
                v2ray_rs_process::relocated_helper_path()
            } else {
                helper.to_path_buf()
            },
            antidpi_path: if relocating && antidpi_relocates {
                v2ray_rs_process::relocated_antidpi_path()
            } else {
                antidpi.to_path_buf()
            },
        }
    }
}

/// Whether a fresh post-grant probe verifies every target the grant offered
/// to repair, captured before elevation: the same backend holds
/// CAP_NET_ADMIN, no needed route-helper reason remains on the same resolved
/// helper path, and an anti-DPI helper the grant targeted ends up ready at
/// the expected destination, relocated or in place. Package guidance or a
/// still-fixable copy the grant never targeted is not a failed repair: the
/// grant never promised to touch it; only a terminal verdict is.
fn grant_confirmed(
    backend_caps: Option<&Result<bool, String>>,
    helper_needed: bool,
    antidpi_targeted: bool,
    targets: &GrantTargets,
    probe: &HelperProbe,
) -> bool {
    if !matches!(backend_caps, Some(Ok(true))) {
        return false;
    }
    if probe.helper != targets.helper_path {
        return false;
    }
    if helper_needed
        .then(|| helper_install_reason(probe))
        .flatten()
        .is_some()
    {
        return false;
    }
    let antidpi = antidpi_install_reason(&probe.antidpi);
    if antidpi_targeted {
        return probe.antidpi.path == targets.antidpi_path
            && matches!(antidpi, AntidpiVerdict::Ready);
    }
    !matches!(antidpi, AntidpiVerdict::Terminal(_))
}

pub(super) fn build_tun_page(
    state: &Rc<RefCell<AppSettings>>,
    cb: &SettingsCallback,
    observers: &SettingsObservers,
    toast_cb: &ToastCallback,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(tr("TUN"))
        .icon_name("network-vpn-symbolic")
        .build();
    page.set_name(Some(TUN_PAGE_NAME));

    let backend = state.borrow().backend.backend_type;
    let is_v2ray = backend == BackendType::V2ray;

    // --- Primary group ------------------------------------------------------
    let primary = adw::PreferencesGroup::builder()
        .title(tr("TUN mode"))
        .description(tr("Route all system traffic through the active proxy"))
        .build();

    let enable_row = adw::SwitchRow::builder()
        .title(tr("Enable TUN"))
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

    // Both generators act on this, so it does not belong with the sing-box-only
    // advanced rows: xray gets a udp/53 routing rule plus a `dns` outbound.
    let hijack_row = adw::ComboRow::builder()
        .title("DNS hijack")
        .subtitle(
            "Hijack routes captured DNS to the backend resolver; native and disabled \
             currently behave the same",
        )
        .model(&gtk::StringList::new(&["hijack", "native", "disabled"]))
        .selected(hijack_to_index(state.borrow().tun.dns_hijack))
        .build();
    primary.add(&hijack_row);
    page.add(&primary);

    // --- Advanced -----------------------------------------------------------
    let advanced_group = adw::PreferencesGroup::new();
    let advanced = adw::ExpanderRow::builder()
        .title("Advanced")
        .subtitle("Stack, routing, DNS, and exclusions")
        .build();

    let advanced_note = adw::ActionRow::builder()
        .title("Stack applies to sing-box only")
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
        .subtitle(
            "Applies to sing-box and xray. With xray, traffic is blocked while the tunnel \
             reconnects, and IPv6 is blocked without an IPv6 tunnel address",
        )
        .active(state.borrow().tun.strict_route)
        .build();
    advanced.add_row(&strict_row);

    advanced_group.add(&advanced);
    page.add(&advanced_group);

    // --- Excluded routes ----------------------------------------------------
    let routes_group = adw::PreferencesGroup::builder()
        .title("Excluded routes")
        .description(EXCLUDED_ROUTES_DESCRIPTION)
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
        let toast_cb = toast_cb.clone();
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
                    let toast_cb = toast_cb.clone();
                    let render_slot = render_slot.clone();
                    delete.connect_clicked(move |_| {
                        if let Err(err) = apply_tun_mutation(&state, &cb, |s| {
                            if index < s.tun.exclude_routes.len() {
                                s.tun.exclude_routes.remove(index);
                            }
                            Ok(())
                        }) {
                            toast_cb(&err);
                        }
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
        let toast_cb = toast_cb.clone();
        add_row.connect_activated(move |_| {
            let state = state.clone();
            let cb = cb.clone();
            let render_routes = render_routes.clone();
            present_add_dialog(
                "Add excluded route",
                "CIDR (e.g. 192.168.0.0/16)",
                &toast_cb,
                move |value| validate_exclude_route(value).map_err(|err| err.to_string()),
                Rc::new(move |value: &str| {
                    apply_tun_mutation(&state, &cb, |s| {
                        s.tun.exclude_routes.push(value.to_string());
                        Ok(())
                    })
                }),
                render_routes,
            );
        });
    }

    // --- Excluded domains ---------------------------------------------------
    let domains_group = adw::PreferencesGroup::builder()
        .title("Excluded domains")
        .description(excluded_domains_description(backend))
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
        let toast_cb = toast_cb.clone();
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
                    let toast_cb = toast_cb.clone();
                    let render_slot = render_slot.clone();
                    delete.connect_clicked(move |_| {
                        if let Err(err) = apply_tun_mutation(&state, &cb, |s| {
                            if index < s.tun.exclude_domains.len() {
                                s.tun.exclude_domains.remove(index);
                            }
                            Ok(())
                        }) {
                            toast_cb(&err);
                        }
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
        let toast_cb = toast_cb.clone();
        add_domain_row.connect_activated(move |_| {
            let state = state.clone();
            let cb = cb.clone();
            let render_domains = render_domains.clone();
            present_add_dialog(
                "Add excluded domain",
                "Domain suffix (e.g. example.com)",
                &toast_cb,
                |value| validate_domain_pattern(value).map_err(|err| err.to_string()),
                Rc::new(move |value: &str| {
                    apply_tun_mutation(&state, &cb, |s| {
                        s.tun.exclude_domains.push(value.to_string());
                        Ok(())
                    })
                }),
                render_domains,
            );
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
        let toast_cb = toast_cb.clone();
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
                    let toast_cb = toast_cb.clone();
                    let render_slot = render_slot.clone();
                    delete.connect_clicked(move |_| {
                        if let Err(err) = apply_tun_mutation(&state, &cb, |s| {
                            if index < s.tun.exclude_processes.len() {
                                s.tun.exclude_processes.remove(index);
                            }
                            Ok(())
                        }) {
                            toast_cb(&err);
                        }
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
        let toast_cb = toast_cb.clone();
        add_app_row.connect_activated(move |_| {
            let state = state.clone();
            let cb = cb.clone();
            let render_apps = render_apps.clone();
            present_add_dialog(
                "Add excluded application",
                "Process name (e.g. cloudflared)",
                &toast_cb,
                |value| {
                    if value.is_empty() {
                        return Err("Process name must not be empty".to_string());
                    }
                    if value.contains('/') || value.contains('\\') {
                        return Err("Process name must not contain path separators".to_string());
                    }
                    Ok(())
                },
                Rc::new(move |value: &str| {
                    apply_tun_mutation(&state, &cb, |s| {
                        s.tun.exclude_processes.push(value.to_string());
                        Ok(())
                    })
                }),
                render_apps,
            );
        });
    }

    // --- Run with bypass (xray only) ----------------------------------------
    let run_group = adw::PreferencesGroup::builder()
        .title("Run with bypass")
        .description("Launch a command routed outside the xray TUN")
        .visible(backend == BackendType::Xray)
        .build();

    // Wrapper resolution stats the filesystem, so it waits for the worker:
    // the row starts hidden and the probe result decides its final state.
    let run_note = adw::ActionRow::builder()
        .title("Wrapper not available")
        .subtitle("Checking the v2ray-rs-bypass system user...")
        .visible(false)
        .build();
    run_group.add(&run_note);

    let run_entry = adw::EntryRow::builder()
        .title("Command (e.g. cloudflared tunnel run)")
        .sensitive(false)
        .build();
    let launch_button = gtk::Button::builder()
        .label("Launch")
        .valign(gtk::Align::Center)
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    run_entry.add_suffix(&launch_button);
    run_group.add(&run_entry);
    page.add(&run_group);

    {
        let run_note = run_note.clone();
        let run_entry = run_entry.clone();
        let launch_button = launch_button.clone();
        glib::MainContext::default().spawn_local(async move {
            let probe = tokio::task::spawn_blocking(|| {
                let run_bin = v2ray_rs_process::run_path();
                let wrapper_on_disk = run_bin.is_file()
                    || std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                        .any(|dir| dir.join(&run_bin).is_file());
                let bypass_user_exists = wrapper_on_disk
                    && nix::unistd::User::from_name(v2ray_rs_process::BYPASS_USER)
                        .ok()
                        .flatten()
                        .is_some();
                (wrapper_on_disk, bypass_user_exists)
            })
            .await;

            match probe {
                Ok((_, true)) => {
                    run_note.set_visible(false);
                    run_entry.set_sensitive(true);
                    launch_button.set_sensitive(true);
                }
                Ok((true, false)) => {
                    run_note.set_subtitle(
                        "The v2ray-rs-bypass system user is missing; install the distribution package",
                    );
                    run_note.set_visible(true);
                }
                _ => {
                    run_note.set_subtitle("Install v2ray-rs-run and grant privileges first");
                    run_note.set_visible(true);
                }
            }
        });
    }

    {
        let toast_cb = toast_cb.clone();
        let run_entry = run_entry.clone();
        launch_button.connect_clicked(move |_| {
            let text = run_entry.text().trim().to_string();
            if text.is_empty() {
                return;
            }
            let args = match glib::shell_parse_argv(&text) {
                Ok(args) => args,
                Err(err) => {
                    toast_cb(&format!("Invalid command: {err}"));
                    return;
                }
            };
            if args.is_empty() {
                return;
            }
            // run_path() stats the filesystem; resolve it off the GTK thread.
            let run_bin = tokio::task::spawn_blocking(v2ray_rs_process::run_path);
            let toast_cb = toast_cb.clone();
            glib::MainContext::default().spawn_local(async move {
                let run_bin = match run_bin.await {
                    Ok(run_bin) => run_bin,
                    Err(_) => {
                        toast_cb("Launch failed: task aborted");
                        return;
                    }
                };
                let mut cmd = tokio::process::Command::new(run_bin);
                cmd.args(&args);
                match cmd.spawn() {
                    Ok(mut child) => {
                        let toast_wait = toast_cb.clone();
                        glib::MainContext::default().spawn_local(async move {
                            match child.wait().await {
                                Ok(status) if status.success() => {}
                                Ok(status) => {
                                    toast_wait(&format!("Command exited with {status}"));
                                }
                                Err(err) => toast_wait(&format!("Command failed: {err}")),
                            }
                        });
                        toast_cb("Launched via bypass wrapper.");
                    }
                    Err(err) => toast_cb(&format!("Launch failed: {err}")),
                }
            });
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
        // The button's click handler strongly owns this closure, so the widget
        // edges stay weak: a strong capture would cycle through the handler,
        // and through the row that owns the button, and leak the page.
        let cap_row = cap_row.downgrade();
        let grant_button = grant_button.downgrade();
        let generation = Rc::new(Cell::new(0_u64));
        let running = Rc::new(Cell::new(false));
        let pending = Rc::new(Cell::new(false));
        // The resumable spawn: while a probe is in flight, further requests
        // only set `pending`, and the in-flight completion re-spawns once.
        // The slot back-references the closure weakly: a strong one would
        // cycle `spawn` through its own slot and leak the captured widgets.
        let spawn_refresh = Rc::new(RefCell::new(None::<Weak<dyn Fn()>>));
        let spawn_refresh_ref = spawn_refresh.clone();
        let spawn: Rc<dyn Fn()> = Rc::new(move || {
            if running.get() {
                pending.set(true);
                return;
            }
            running.set(true);
            let request = generation.get().wrapping_add(1);
            generation.set(request);
            let generation = generation.clone();
            let running = running.clone();
            let pending = pending.clone();
            let cap_row = cap_row.clone();
            let grant_button = grant_button.clone();
            let state = state.clone();
            let spawn_refresh = spawn_refresh_ref.clone();
            glib::MainContext::default().spawn_local(async move {
                let helper_needed = state.borrow().backend.backend_type == BackendType::Xray;
                let backend_path = state.borrow().backend.binary_path.clone();
                let probed = tokio::task::spawn_blocking(move || {
                    let backend_caps = backend_path.as_ref().map(|path| {
                        v2ray_rs_process::has_net_admin(path).map_err(|err| err.to_string())
                    });
                    let helper = probe_helper();
                    (backend_caps, helper)
                })
                .await
                .ok();
                // An obsolete response: a newer refresh superseded this one.
                if generation.get() != request {
                    return;
                }
                running.set(false);
                if pending.take() {
                    if let Some(spawn) = spawn_refresh
                        .borrow()
                        .as_ref()
                        .and_then(|weak| weak.upgrade())
                    {
                        spawn();
                    }
                    return;
                }
                let (message, grant) = match probed {
                    None => (
                        "Cannot read TUN capability state: probe task aborted".to_string(),
                        false,
                    ),
                    Some((backend_caps, helper)) => {
                        capability_status(backend_caps.as_ref(), helper_needed, &helper)
                    }
                };
                let (Some(cap_row), Some(grant_button)) =
                    (cap_row.upgrade(), grant_button.upgrade())
                else {
                    return;
                };
                cap_row.set_subtitle(&message);
                grant_button.set_visible(grant);
            });
        });
        *spawn_refresh.borrow_mut() = Some(Rc::downgrade(&spawn));
        spawn
    };
    refresh_caps();

    {
        let state = state.clone();
        let refresh = refresh_caps.clone();
        let toast_cb = toast_cb.clone();
        grant_button.connect_clicked(move |btn| {
            let backend_path = state.borrow().backend.binary_path.clone();
            let Some(backend_path) = backend_path else {
                return;
            };
            // grant() blocks on the polkit dialog; run it off the GTK thread so
            // the whole app doesn't freeze until the prompt is answered.
            btn.set_sensitive(false);
            let btn = btn.clone();
            let refresh = refresh.clone();
            let toast_cb = toast_cb.clone();
            // The closure may fire again, so the async block moves clones:
            // `state` stays owned by the `Fn` closure, and `backend_path`
            // flows through the preparation task into the grant task.
            let state = state.clone();
            glib::MainContext::default().spawn_local(async move {
                // Helper resolution, the target capture, and the pre-grant
                // probe all hit the filesystem, so they run beside the grant
                // itself. The capture mirrors `plan_grant`: confirmed success
                // requires these same post-grant destinations healthy on the
                // fresh probe.
                let prepared = tokio::task::spawn_blocking(move || {
                    let helper = v2ray_rs_process::helper_path();
                    // The anti-DPI helper rides along in the same elevation
                    // when it is installed; grant() skips absent targets
                    // itself.
                    let antidpi = v2ray_rs_process::antidpi_path();
                    let targets = GrantTargets::for_plan(
                        backend_path.clone(),
                        &helper,
                        &antidpi,
                        v2ray_rs_process::relocation_required(),
                        v2ray_rs_process::bundled_antidpi_path().is_some(),
                    );
                    // An anti-DPI helper that was grant-repairable before
                    // elevation becomes a required target: the re-probe must
                    // see it ready and still managed.
                    let antidpi_targeted = matches!(
                        antidpi_install_reason(&probe_antidpi()),
                        AntidpiVerdict::Fixable(_)
                    );
                    (backend_path, helper, antidpi, targets, antidpi_targeted)
                })
                .await;
                let (backend_path, helper, antidpi, targets, antidpi_targeted) = match prepared {
                    Ok(prepared) => prepared,
                    // Fail closed if the capture probe cannot run.
                    Err(_) => {
                        btn.set_sensitive(true);
                        toast_cb("Cannot probe helpers: task aborted");
                        refresh();
                        return;
                    }
                };
                let result = tokio::task::spawn_blocking(move || {
                    v2ray_rs_process::grant(&backend_path, &helper, Some(&antidpi))
                        .map_err(|e| e.to_string())
                })
                .await;
                btn.set_sensitive(true);
                match result {
                    Ok(Ok(())) => {
                        // Claim success only after a fresh probe confirms
                        // the grant repaired exactly the captured targets;
                        // the row refreshes from the same post-grant state
                        // either way, with one toast.
                        let helper_needed =
                            state.borrow().backend.backend_type == BackendType::Xray;
                        let (confirmed, message) = tokio::task::spawn_blocking(move || {
                            let backend_caps =
                                v2ray_rs_process::has_net_admin(&targets.backend_path)
                                    .map_err(|err| err.to_string());
                            let probe = probe_helper();
                            let confirmed = grant_confirmed(
                                Some(&backend_caps),
                                helper_needed,
                                antidpi_targeted,
                                &targets,
                                &probe,
                            );
                            let (message, _) =
                                capability_status(Some(&backend_caps), helper_needed, &probe);
                            (confirmed, message)
                        })
                        .await
                        .unwrap_or_else(|_| {
                            (false, "capability verification task aborted".to_string())
                        });
                        if confirmed {
                            toast_cb("TUN privileges granted.");
                        } else {
                            toast_cb(&format!("Grant did not fully verify: {message}"));
                        }
                    }
                    Ok(Err(err)) => toast_cb(&format!("Grant failed: {err}")),
                    Err(_) => toast_cb("Grant failed: elevation task aborted"),
                }
                refresh();
            });
        });
    }

    // --- Handlers -----------------------------------------------------------
    {
        let state = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        let warned = Rc::new(Cell::new(false));
        let guard = Rc::new(Cell::new(false));
        enable_row.connect_active_notify(move |row| {
            if guard.get() {
                return;
            }
            let enabled = row.is_active();
            if let Err(err) = apply_tun_mutation(&state, &cb, |s| {
                s.tun.enabled = enabled;
                Ok(())
            }) {
                toast_cb(&format!("Failed to save TUN setting: {err}"));
                with_rollback_guard(&guard, || {
                    row.set_active(state.borrow().tun.enabled);
                });
                return;
            }
            if enabled && !warned.replace(true) {
                toast_cb("TUN routes all system traffic through the active proxy.");
            }
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        iface_row.connect_apply(move |row| {
            let value = row.text().trim().to_string();
            match validate_tun_interface_name(&value) {
                Ok(()) => {
                    row.remove_css_class("error");
                    if let Err(err) = apply_tun_mutation(&state, &cb, |s| {
                        s.tun.interface_name = value.clone();
                        Ok(())
                    }) {
                        toast_cb(&format!("Failed to save TUN setting: {err}"));
                        row.set_text(&state.borrow().tun.interface_name);
                    }
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
        let toast_cb = toast_cb.clone();
        address_row.connect_apply(move |row| {
            let value = row.text().trim().to_string();
            match validate_ip_cidr(&value) {
                Ok(()) => {
                    row.remove_css_class("error");
                    if let Err(err) = apply_tun_mutation(&state, &cb, |s| {
                        s.tun.address_v4 = value.clone();
                        Ok(())
                    }) {
                        toast_cb(&format!("Failed to save TUN setting: {err}"));
                        row.set_text(&state.borrow().tun.address_v4);
                    }
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
        let toast_cb = toast_cb.clone();
        let guard = Rc::new(Cell::new(false));
        mtu_row.connect_changed(move |row| {
            if guard.get() {
                return;
            }
            let mtu = row.value() as u16;
            if let Err(err) = apply_tun_mutation(&state, &cb, |s| {
                s.tun.mtu = mtu;
                Ok(())
            }) {
                toast_cb(&format!("Failed to save TUN setting: {err}"));
                with_rollback_guard(&guard, || {
                    row.set_value(state.borrow().tun.mtu as f64);
                });
            }
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        let guard = Rc::new(Cell::new(false));
        stack_row.connect_selected_notify(move |row| {
            if guard.get() {
                return;
            }
            let stack = index_to_stack(row.selected());
            if let Err(err) = apply_tun_mutation(&state, &cb, |s| {
                s.tun.stack = stack;
                Ok(())
            }) {
                toast_cb(&format!("Failed to save TUN setting: {err}"));
                with_rollback_guard(&guard, || {
                    row.set_selected(stack_to_index(state.borrow().tun.stack));
                });
            }
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        let guard = Rc::new(Cell::new(false));
        strict_row.connect_active_notify(move |row| {
            if guard.get() {
                return;
            }
            let active = row.is_active();
            if let Err(err) = apply_tun_mutation(&state, &cb, |s| {
                s.tun.strict_route = active;
                Ok(())
            }) {
                toast_cb(&format!("Failed to save TUN setting: {err}"));
                with_rollback_guard(&guard, || {
                    row.set_active(state.borrow().tun.strict_route);
                });
            }
        });
    }
    {
        let state = state.clone();
        let cb = cb.clone();
        let toast_cb = toast_cb.clone();
        let guard = Rc::new(Cell::new(false));
        hijack_row.connect_selected_notify(move |row| {
            if guard.get() {
                return;
            }
            let mode = index_to_hijack(row.selected());
            if let Err(err) = apply_tun_mutation(&state, &cb, |s| {
                s.tun.dns_hijack = mode;
                Ok(())
            }) {
                toast_cb(&format!("Failed to save TUN setting: {err}"));
                with_rollback_guard(&guard, || {
                    row.set_selected(hijack_to_index(state.borrow().tun.dns_hijack));
                });
            }
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
            strict_row.set_sensitive(strict_route_applies(backend));
            hijack_row.set_sensitive(backend != BackendType::V2ray);
            routes_group.set_sensitive(backend != BackendType::V2ray);
            domains_group.set_sensitive(backend != BackendType::V2ray);
            domains_group.set_description(Some(excluded_domains_description(backend)));
            apps_group.set_sensitive(singbox_only);
            refresh_caps();
        });
    }
    // Apply the initial backend gating for the advanced rows.
    let singbox_only = backend == BackendType::SingBox;
    stack_row.set_sensitive(singbox_only);
    strict_row.set_sensitive(strict_route_applies(backend));
    hijack_row.set_sensitive(backend != BackendType::V2ray);
    routes_group.set_sensitive(backend != BackendType::V2ray);
    domains_group.set_sensitive(backend != BackendType::V2ray);
    apps_group.set_sensitive(singbox_only);

    page
}

fn strict_route_applies(backend: BackendType) -> bool {
    matches!(backend, BackendType::SingBox | BackendType::Xray)
}

// User-edited exclusions plus the automatic ones merged at config generation.
const EXCLUDED_ROUTES_DESCRIPTION: &str = "CIDRs routed outside the tunnel; the host's own attached networks are excluded \
     automatically; applies on next connect";

// xray has no TUN-level domain bypass: matching flows enter the device and are
// only sent out directly by its routing.
fn excluded_domains_description(backend: BackendType) -> &'static str {
    match backend {
        BackendType::Xray => {
            "Matching traffic still passes through the tunnel and is sent directly by xray; applies on next connect"
        }
        BackendType::SingBox | BackendType::V2ray => {
            "Domain suffixes that bypass the tunnel; applies on next connect"
        }
    }
}

type ApplyMutation = Rc<dyn Fn(&str) -> Result<(), String>>;

/// Small add-item dialog. Validation failures stay inline with the dialog
/// open; a mutation failure (the value itself was accepted but the settings
/// write rejected it) is toasted and the dialog stays open on the bad value.
fn present_add_dialog(
    title: &str,
    entry_title: &str,
    toast_cb: &ToastCallback,
    validate: impl Fn(&str) -> Result<(), String> + 'static,
    apply: ApplyMutation,
    applied: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::builder()
        .title(title)
        .width_request(420)
        .build();

    let entry = adw::EntryRow::builder().title(entry_title).build();
    let error_label = gtk::Label::builder()
        .visible(false)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["error"])
        .build();
    let group = adw::PreferencesGroup::new();
    group.add(&entry);

    let cancel = gtk::Button::builder().label("Cancel").build();
    let add = gtk::Button::builder()
        .label("Add")
        .css_classes(["suggested-action"])
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&group);
    content.append(&error_label);
    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .halign(gtk::Align::End)
        .build();
    buttons.append(&cancel);
    buttons.append(&add);
    content.append(&buttons);
    dialog.set_child(Some(&content));
    dialog.set_default_widget(Some(&add));

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let dialog = dialog.clone();
        let entry = entry.clone();
        let error_label = error_label.clone();
        let toast_cb = toast_cb.clone();
        // gtk handlers are Fn; every closure here is callable repeatedly.
        add.connect_clicked(move |_| {
            error_label.set_visible(false);
            let value = entry.text().trim().to_string();
            if let Err(err) = validate(&value) {
                error_label.set_text(&err);
                error_label.set_visible(true);
                return;
            }
            if let Err(err) = apply(&value) {
                toast_cb(&format!("Failed to save: {err}"));
                return;
            }
            dialog.close();
            applied();
        });
    }
    dialog.present(crate::active_window().as_ref());
}

/// Restores a widget after a rejected mutation without letting the restore
/// re-enter its own change handler.
fn with_rollback_guard(guard: &Cell<bool>, rollback: impl FnOnce()) {
    if guard.replace(true) {
        return;
    }
    rollback();
    guard.set(false);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_route_row_sensitive_for_xray() {
        assert!(strict_route_applies(BackendType::Xray));
        assert!(strict_route_applies(BackendType::SingBox));
        assert!(!strict_route_applies(BackendType::V2ray));
    }

    #[test]
    fn excluded_routes_description_names_next_connect() {
        assert!(EXCLUDED_ROUTES_DESCRIPTION.contains("outside the tunnel"));
        assert!(EXCLUDED_ROUTES_DESCRIPTION.ends_with("applies on next connect"));
    }

    #[test]
    fn excluded_domains_description_per_backend() {
        let xray = excluded_domains_description(BackendType::Xray);
        assert!(xray.contains("still passes through the tunnel"));
        assert!(!xray.contains("bypass"));
        assert!(xray.ends_with("applies on next connect"));
        let bypass = "Domain suffixes that bypass the tunnel; applies on next connect";
        assert_eq!(excluded_domains_description(BackendType::SingBox), bypass);
        assert_eq!(excluded_domains_description(BackendType::V2ray), bypass);
    }

    fn probe(
        access: v2ray_rs_process::HelperAccess,
        capability: Option<Result<bool, String>>,
        stale: bool,
        missing_installable: bool,
    ) -> HelperProbe {
        HelperProbe {
            helper: PathBuf::from("/usr/local/bin/v2ray-rs-netctl"),
            access,
            capability,
            stale,
            missing_installable,
            antidpi: antidpi_probe(
                v2ray_rs_process::HelperAccess::Ready,
                Some(Ok(true)),
                false,
                true,
            ),
        }
    }

    fn antidpi_probe(
        access: v2ray_rs_process::HelperAccess,
        capability: Option<Result<bool, String>>,
        stale: bool,
        grant_eligible: bool,
    ) -> AntiDpiProbe {
        AntiDpiProbe {
            path: PathBuf::from("/usr/local/bin/v2ray-rs-antidpi"),
            access,
            capability,
            stale,
            grant_eligible,
        }
    }

    #[test]
    fn ready_helper_with_capability_is_silent() {
        assert!(
            helper_install_reason(&probe(
                v2ray_rs_process::HelperAccess::Ready,
                Some(Ok(true)),
                false,
                false,
            ))
            .is_none()
        );
    }

    #[test]
    fn stale_relocated_helper_offers_grant() {
        let (message, grant) = helper_install_reason(&probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            true,
            false,
        ))
        .expect("stale helper has a reason");
        assert!(message.contains("out of date"));
        assert!(grant);
    }

    #[test]
    fn missing_helper_capability_offers_grant() {
        let (message, grant) = helper_install_reason(&probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(false)),
            false,
            false,
        ))
        .expect("missing capability has a reason");
        assert!(message.contains("lacks CAP_NET_ADMIN"));
        assert!(grant);
    }

    #[test]
    fn failed_capability_probe_shows_error_without_grant() {
        let (message, grant) = helper_install_reason(&probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Err("getcap crashed".to_string())),
            false,
            false,
        ))
        .expect("probe error has a reason");
        assert!(message.contains("Cannot read capabilities"));
        assert!(message.contains("getcap crashed"));
        assert!(!grant);
    }

    #[test]
    fn enrollment_and_permissions_offer_grant() {
        for access in [
            v2ray_rs_process::HelperAccess::EnrollmentRequired,
            v2ray_rs_process::HelperAccess::Permissions,
        ] {
            let (message, grant) = helper_install_reason(&probe(access, None, false, false))
                .expect("repairable access has a reason");
            assert!(grant, "{message}");
        }
    }

    #[test]
    fn relogin_noexec_and_probe_failure_show_error_without_grant() {
        for access in [
            v2ray_rs_process::HelperAccess::ReloginRequired,
            v2ray_rs_process::HelperAccess::NoExec,
            v2ray_rs_process::HelperAccess::ProbeFailed("mount unreadable".to_string()),
        ] {
            let (message, grant) = helper_install_reason(&probe(access, None, false, false))
                .expect("nonrepairable access has a reason");
            assert!(!grant, "{message}");
        }
    }

    #[test]
    fn missing_helper_grant_tracks_relocation_source() {
        let (message, grant) = helper_install_reason(&probe(
            v2ray_rs_process::HelperAccess::Missing,
            None,
            false,
            true,
        ))
        .expect("missing helper has a reason");
        assert!(message.contains("needs installing"));
        assert!(grant);

        let (message, grant) = helper_install_reason(&probe(
            v2ray_rs_process::HelperAccess::Missing,
            None,
            false,
            false,
        ))
        .expect("missing helper has a reason");
        assert!(message.contains("distribution package"));
        assert!(!grant);
    }

    #[test]
    fn nonrepairable_helper_blocker_wins_over_backend_grant() {
        let helper = probe(
            v2ray_rs_process::HelperAccess::ReloginRequired,
            None,
            false,
            false,
        );
        let backend_missing = Ok::<bool, String>(false);
        let (message, grant) = capability_status(Some(&backend_missing), true, &helper);
        assert!(message.contains("log out and back in"));
        assert!(!grant);
    }

    #[test]
    fn helper_skipped_when_backend_does_not_need_it() {
        let helper = probe(
            v2ray_rs_process::HelperAccess::ReloginRequired,
            None,
            false,
            false,
        );
        let backend_ready = Ok::<bool, String>(true);
        let (message, grant) = capability_status(Some(&backend_ready), false, &helper);
        assert_eq!(message, "CAP_NET_ADMIN granted");
        assert!(!grant);
    }

    #[test]
    fn backend_decisions_unchanged_by_ready_helper() {
        let helper = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        let backend_missing = Ok::<bool, String>(false);
        let (message, grant) = capability_status(Some(&backend_missing), true, &helper);
        assert_eq!(message, "Backend lacks CAP_NET_ADMIN");
        assert!(grant);

        let (message, grant) = capability_status(None, true, &helper);
        assert_eq!(message, "Configure a backend binary first");
        assert!(!grant);

        let backend_probe = Err::<bool, String>("getcap not installed".to_string());
        let (message, grant) = capability_status(Some(&backend_probe), true, &helper);
        assert!(message.contains("Cannot read capabilities"));
        assert!(!grant);
    }

    /// The dialog surfaces exactly this error string inline, and
    /// `apply_tun_mutation` must have left the settings untouched.
    #[test]
    fn rejected_exclusion_is_reported_and_not_saved() {
        let state = Rc::new(RefCell::new(AppSettings::default()));
        let notifications = Rc::new(Cell::new(0_u32));
        let cb: SettingsCallback = {
            let notifications = notifications.clone();
            Rc::new(move |_| notifications.set(notifications.get() + 1))
        };

        let err = apply_tun_mutation(&state, &cb, |s| {
            s.tun.exclude_routes.push("0.0.0.0/0".to_string());
            Ok(())
        })
        .expect_err("an all-address exclusion must be rejected");

        assert!(err.contains("0.0.0.0/0"), "{err}");
        assert!(
            state.borrow().tun.exclude_routes.is_empty(),
            "rejected input must not mutate settings"
        );
        assert_eq!(
            notifications.get(),
            0,
            "rejected input must not notify observers"
        );
    }

    #[test]
    fn overflowing_exclusion_is_reported_and_not_saved() {
        let state = Rc::new(RefCell::new(AppSettings::default()));
        for octet in 0..=255u8 {
            state
                .borrow_mut()
                .tun
                .exclude_routes
                .push(format!("10.0.{octet}.0/24"));
        }
        let cb: SettingsCallback = Rc::new(|_| {});

        let err = apply_tun_mutation(&state, &cb, |s| {
            s.tun.exclude_routes.push("10.1.0.0/24".to_string());
            Ok(())
        })
        .expect_err("entry 257 must be rejected");

        assert!(err.contains("at most 256"), "{err}");
        assert_eq!(state.borrow().tun.exclude_routes.len(), 256);
    }

    #[test]
    fn antidpi_caps_missing_offers_grant_when_backend_and_helper_are_healthy() {
        let mut full = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        full.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(false)),
            false,
            true,
        );
        let backend_ready = Ok::<bool, String>(true);

        let (message, grant) = capability_status(Some(&backend_ready), true, &full);
        assert_eq!(
            message,
            tr("Anti-DPI helper lacks cap_net_admin or cap_net_raw; grant again")
        );
        assert!(grant);

        // Same verdict with sing-box, where no route helper is consulted.
        let (message, grant) = capability_status(Some(&backend_ready), false, &full);
        assert_eq!(
            message,
            tr("Anti-DPI helper lacks cap_net_admin or cap_net_raw; grant again")
        );
        assert!(grant);
    }

    #[test]
    fn unreadable_antidpi_probe_reports_without_grant() {
        let mut full = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        // An unreadable probe on a grant-eligible helper is unverified
        // absence: no grant, and it still outranks a fixable backend.
        full.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Err("getcap crashed".to_string())),
            false,
            true,
        );
        let backend_missing = Ok::<bool, String>(false);
        let (message, grant) = capability_status(Some(&backend_missing), true, &full);
        let prefix = tr("Cannot read anti-DPI capabilities");
        assert!(message.starts_with(&prefix), "{message}");
        assert!(message.contains("getcap crashed"), "{message}");
        assert!(!grant);

        // The same failure on an unmanaged copy is package repair: it
        // never suppresses the backend grant, and the guidance rides
        // along in the message.
        full.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::ProbeFailed("metadata unreadable".to_string()),
            None,
            false,
            false,
        );
        let (message, grant) = capability_status(Some(&backend_missing), true, &full);
        assert!(message.contains("Backend lacks CAP_NET_ADMIN"), "{message}");
        assert!(
            message.contains("install the distribution package"),
            "{message}"
        );
        assert!(grant);
    }

    #[test]
    fn ready_antidpi_changes_no_backend_or_helper_message() {
        let helper = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        let backend_ready = Ok::<bool, String>(true);
        let (message, grant) = capability_status(Some(&backend_ready), true, &helper);
        assert_eq!(message, "CAP_NET_ADMIN granted");
        assert!(!grant);

        let backend_missing = Ok::<bool, String>(false);
        let (message, grant) = capability_status(Some(&backend_missing), true, &helper);
        assert_eq!(message, "Backend lacks CAP_NET_ADMIN");
        assert!(grant);
    }

    #[test]
    fn stale_antidpi_helper_offers_grant() {
        let mut full = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        full.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            true,
            true,
        );
        let backend_ready = Ok::<bool, String>(true);
        let (message, grant) = capability_status(Some(&backend_ready), true, &full);
        assert_eq!(
            message,
            tr("Installed anti-DPI helper is out of date; grant again")
        );
        assert!(grant);
    }

    #[test]
    fn antidpi_missing_helper_grant_tracks_the_install_source() {
        let mut full = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        let backend_ready = Ok::<bool, String>(true);

        full.antidpi = antidpi_probe(v2ray_rs_process::HelperAccess::Missing, None, false, true);
        let (message, grant) = capability_status(Some(&backend_ready), true, &full);
        assert_eq!(
            message,
            tr("Anti-DPI helper needs installing before the anti-DPI engine can start")
        );
        assert!(grant);

        full.antidpi = antidpi_probe(v2ray_rs_process::HelperAccess::Missing, None, false, false);
        let (message, grant) = capability_status(Some(&backend_ready), true, &full);
        assert!(message.contains("not found"), "{message}");
        assert!(!grant);
    }

    #[test]
    fn antidpi_enrollment_and_permissions_offer_grant_terminal_ones_do_not() {
        let backend_ready = Ok::<bool, String>(true);
        for (access, expect_grant) in [
            (v2ray_rs_process::HelperAccess::EnrollmentRequired, true),
            (v2ray_rs_process::HelperAccess::Permissions, true),
            (v2ray_rs_process::HelperAccess::ReloginRequired, false),
            (v2ray_rs_process::HelperAccess::NoExec, false),
        ] {
            let mut full = probe(
                v2ray_rs_process::HelperAccess::Ready,
                Some(Ok(true)),
                false,
                false,
            );
            full.antidpi = antidpi_probe(access, None, false, true);
            let (message, grant) = capability_status(Some(&backend_ready), true, &full);
            assert!(message.contains("anti-DPI"), "{message}");
            assert_eq!(grant, expect_grant, "{message}");
        }
    }

    /// TUN-only install (no route helper consulted): backend caps missing
    /// plus a broken managed anti-DPI helper. The managed helper rides
    /// along with the backend grant instead of outranking it.
    #[test]
    fn tun_only_missing_backend_caps_with_broken_managed_helper() {
        let mut full = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        full.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(false)),
            false,
            true,
        );
        let backend_missing = Ok::<bool, String>(false);
        let (message, grant) = capability_status(Some(&backend_missing), false, &full);
        assert!(message.contains("Backend lacks CAP_NET_ADMIN"), "{message}");
        assert!(
            message.contains("lacks cap_net_admin or cap_net_raw"),
            "{message}"
        );
        assert!(grant);
    }

    /// A PATH-only unmanaged copy is package repair: no grant for the
    /// helper itself, and it never suppresses a fixable backend grant.
    #[test]
    fn path_only_unmanaged_helper_shows_package_guidance() {
        let mut full = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        let guidance =
            tr("Anti-DPI helper is not managed by v2ray-rs; install the distribution package");

        full.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(false)),
            false,
            false,
        );
        let backend_ready = Ok::<bool, String>(true);
        let (message, grant) = capability_status(Some(&backend_ready), true, &full);
        assert_eq!(message, guidance);
        assert!(!grant);

        let backend_missing = Ok::<bool, String>(false);
        let (message, grant) = capability_status(Some(&backend_missing), true, &full);
        assert!(message.contains("Backend lacks CAP_NET_ADMIN"), "{message}");
        assert!(
            message.contains("install the distribution package"),
            "{message}"
        );
        assert!(grant);

        // An absent helper without a bundled source is package repair too.
        full.antidpi = antidpi_probe(v2ray_rs_process::HelperAccess::Missing, None, false, false);
        let (message, grant) = capability_status(Some(&backend_missing), true, &full);
        assert!(message.contains("not found"), "{message}");
        assert!(message.contains("Backend lacks CAP_NET_ADMIN"), "{message}");
        assert!(grant);
    }

    /// A stale grant-eligible helper offers the grant; a stale unmanaged
    /// copy falls back to package guidance.
    #[test]
    fn grant_eligible_stale_helper_offers_grant() {
        let mut full = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        full.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            true,
            true,
        );
        let backend_ready = Ok::<bool, String>(true);
        let (message, grant) = capability_status(Some(&backend_ready), true, &full);
        assert_eq!(
            message,
            tr("Installed anti-DPI helper is out of date; grant again")
        );
        assert!(grant);

        // The stale managed helper rides along with a backend grant.
        let backend_missing = Ok::<bool, String>(false);
        let (message, grant) = capability_status(Some(&backend_missing), true, &full);
        assert!(message.contains("Backend lacks CAP_NET_ADMIN"), "{message}");
        assert!(message.contains("out of date"), "{message}");
        assert!(grant);

        full.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            true,
            false,
        );
        let (message, grant) = capability_status(Some(&backend_ready), true, &full);
        assert_eq!(
            message,
            tr("Anti-DPI helper is not managed by v2ray-rs; install the distribution package")
        );
        assert!(!grant);
    }

    /// Post-grant verification: confirmed only when exactly the captured
    /// targets verify clean on the fresh probe.
    #[test]
    fn grant_confirmation_requires_verified_targets() {
        let backend_ready = Some(&Ok::<bool, String>(true));
        let full = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        let targets = GrantTargets {
            backend_path: full.helper.clone(),
            helper_path: full.helper.clone(),
            antidpi_path: full.antidpi.path.clone(),
        };
        assert!(grant_confirmed(backend_ready, true, false, &targets, &full));

        // A grant-repairable anti-DPI helper must end up ready.
        let mut capsless = full.clone();
        capsless.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(false)),
            false,
            true,
        );
        assert!(!grant_confirmed(
            backend_ready,
            true,
            true,
            &targets,
            &capsless
        ));
        assert!(grant_confirmed(
            backend_ready,
            true,
            false,
            &targets,
            &capsless
        ));

        // Package guidance for an untargeted unmanaged copy is not a failed
        // repair.
        let mut unmanaged = full.clone();
        unmanaged.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(false)),
            false,
            false,
        );
        assert!(grant_confirmed(
            backend_ready,
            true,
            false,
            &targets,
            &unmanaged
        ));

        // A helper that disappeared or resolved to a different copy does
        // not confirm the repair.
        let mut relocated = full.clone();
        relocated.antidpi.path = PathBuf::from("/home/user/.local/bin/v2ray-rs-antidpi");
        assert!(!grant_confirmed(
            backend_ready,
            true,
            true,
            &targets,
            &relocated
        ));
        let mut moved_helper = full.clone();
        moved_helper.helper = PathBuf::from("/run/user/1000/bin/v2ray-rs-netctl");
        assert!(!grant_confirmed(
            backend_ready,
            true,
            false,
            &targets,
            &moved_helper
        ));

        let unreadable = Err::<bool, String>("getcap missing".to_string());
        assert!(!grant_confirmed(
            Some(&unreadable),
            true,
            false,
            &targets,
            &full
        ));
    }

    /// A relocating grant (AppImage, `nosuid` mount) installs under
    /// `RELOCATE_DIR`, so the captured targets must predict the relocated
    /// destinations: a first-time grant confirms cleanly once the fresh
    /// probe resolves the relocated copies.
    #[test]
    fn relocating_grant_confirmation_targets_relocated_paths() {
        let backend_ready = Some(&Ok::<bool, String>(true));
        let bundled_helper = PathBuf::from("/tmp/appimg/bin/v2ray-rs-netctl");
        let bundled_antidpi = PathBuf::from("/tmp/appimg/bin/v2ray-rs-antidpi");
        let relocated_helper = PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-netctl");
        let relocated_antidpi = PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-antidpi");

        let targets = GrantTargets::for_plan(
            bundled_helper.clone(),
            &bundled_helper,
            &bundled_antidpi,
            true,
            true,
        );
        assert_eq!(targets.helper_path, relocated_helper);
        assert_eq!(targets.antidpi_path, relocated_antidpi);

        // Post-grant probe: both relocated copies ready and managed.
        let mut relocated = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        relocated.helper = relocated_helper.clone();
        relocated.antidpi.path = relocated_antidpi.clone();
        assert!(grant_confirmed(
            backend_ready,
            true,
            true,
            &targets,
            &relocated
        ));
        assert!(grant_confirmed(
            backend_ready,
            false,
            false,
            &targets,
            &relocated
        ));

        // A probe that still resolves the in-place paths does not confirm
        // the relocating grant.
        let mut still_in_place = relocated.clone();
        still_in_place.helper = bundled_helper.clone();
        assert!(!grant_confirmed(
            backend_ready,
            false,
            false,
            &targets,
            &still_in_place
        ));
        let mut stale_antidpi = relocated.clone();
        stale_antidpi.antidpi.path = bundled_antidpi.clone();
        assert!(!grant_confirmed(
            backend_ready,
            false,
            true,
            &targets,
            &stale_antidpi
        ));

        // An in-place plan keeps the resolved paths as the targets.
        let inplace = GrantTargets::for_plan(
            bundled_helper.clone(),
            &bundled_helper,
            &bundled_antidpi,
            false,
            true,
        );
        assert_eq!(inplace.helper_path, bundled_helper);
        assert_eq!(inplace.antidpi_path, bundled_antidpi);

        // Relocating without a bundled anti-DPI source: no anti-DPI target,
        // so a fixable copy the grant never carried cannot fail it.
        let helper_only = GrantTargets::for_plan(
            bundled_helper.clone(),
            &bundled_helper,
            &bundled_antidpi,
            true,
            false,
        );
        assert_eq!(helper_only.antidpi_path, bundled_antidpi);
        let mut fixable = relocated.clone();
        fixable.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(false)),
            false,
            true,
        );
        fixable.antidpi.path = bundled_antidpi.clone();
        assert!(grant_confirmed(
            backend_ready,
            false,
            false,
            &helper_only,
            &fixable
        ));

        // A terminal verdict on the untargeted helper still fails.
        let mut terminal = fixable.clone();
        terminal.antidpi = antidpi_probe(v2ray_rs_process::HelperAccess::NoExec, None, false, true);
        terminal.antidpi.path = bundled_antidpi.clone();
        assert!(!grant_confirmed(
            backend_ready,
            false,
            false,
            &helper_only,
            &terminal
        ));
    }

    /// A relocating grant promises the fixed `RELOCATE_DIR` destinations:
    /// a healthy copy anywhere else — a PATH install, an unrelated
    /// directory, a bare fallback name — is a wrong destination and must
    /// not confirm the grant.
    #[test]
    fn relocating_grant_rejects_wrong_destinations() {
        let backend_ready = Some(&Ok::<bool, String>(true));
        let bundled_helper = PathBuf::from("/tmp/appimg/bin/v2ray-rs-netctl");
        let bundled_antidpi = PathBuf::from("/tmp/appimg/bin/v2ray-rs-antidpi");
        let targets = GrantTargets::for_plan(
            bundled_helper.clone(),
            &bundled_helper,
            &bundled_antidpi,
            true,
            true,
        );

        // A healthy helper installed anywhere but the promised destination
        // is not the grant's repair, whether or not TUN needs the helper.
        let mut wrong_helper = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(true)),
            false,
            false,
        );
        wrong_helper.helper = PathBuf::from("/usr/local/bin/v2ray-rs-netctl");
        assert!(!grant_confirmed(
            backend_ready,
            true,
            true,
            &targets,
            &wrong_helper
        ));
        assert!(!grant_confirmed(
            backend_ready,
            false,
            false,
            &targets,
            &wrong_helper
        ));

        // A healthy targeted anti-DPI helper at the wrong place fails too;
        // rejection is by destination, not by verdict.
        let mut wrong_antidpi = wrong_helper.clone();
        wrong_antidpi.helper = targets.helper_path.clone();
        wrong_antidpi.antidpi.path = PathBuf::from("/usr/local/bin/v2ray-rs-antidpi");
        assert!(!grant_confirmed(
            backend_ready,
            true,
            true,
            &targets,
            &wrong_antidpi
        ));

        // A bare fallback name (nothing installed anywhere resolvable) is
        // equally wrong.
        let mut unresolved = wrong_antidpi.clone();
        unresolved.antidpi.path = PathBuf::from("v2ray-rs-antidpi");
        assert!(!grant_confirmed(
            backend_ready,
            true,
            true,
            &targets,
            &unresolved
        ));

        // An untargeted copy elsewhere was never the grant's promise; it
        // does not fail the grant either — only a terminal verdict would.
        let untargeted = grant_confirmed(backend_ready, false, false, &targets, &wrong_antidpi);
        assert!(untargeted);
    }

    /// A healthy backend with a fixable route helper keeps the grant visible
    /// while an unmanaged anti-DPI helper's package guidance rides along.
    #[test]
    fn route_helper_repair_keeps_unmanaged_antidpi_guidance_visible() {
        let mut helper = probe(
            v2ray_rs_process::HelperAccess::Ready,
            Some(Ok(false)),
            false,
            false,
        );
        helper.antidpi = antidpi_probe(
            v2ray_rs_process::HelperAccess::ProbeFailed("metadata unreadable".to_string()),
            None,
            false,
            false,
        );
        let backend_ready = Ok::<bool, String>(true);
        let (message, grant) = capability_status(Some(&backend_ready), true, &helper);
        assert!(message.contains("lacks CAP_NET_ADMIN"), "{message}");
        assert!(
            message.contains("install the distribution package"),
            "{message}"
        );
        assert!(grant);
    }
}
