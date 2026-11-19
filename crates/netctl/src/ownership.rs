use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use futures::TryStreamExt;
use rtnetlink::packet_core::{Emitable, Parseable};
use rtnetlink::packet_route::link::{InfoKind, LinkAttribute, LinkInfo, LinkMessage};
use rtnetlink::packet_route::route::{RouteAttribute, RouteMessage, RouteMessageBuffer};
use rtnetlink::packet_route::rule::{RuleAttribute, RuleMessage, RuleMessageBuffer};
use rtnetlink::{Handle, IpVersion, LinkDummy, LinkUnspec, RouteMessageBuilder};
use v2ray_rs_tun_session::{Ownership, Phase, RESERVATION_NAME, Resource, Session};

use crate::net;

/// Serializes helper operations on one journal: the lock file lives beside the
/// journal (`<journal>.lock`, created 0600, never through a symlink) and is
/// held for the caller's process lifetime. With no journal (`--force-legacy`
/// operator recovery) there is nothing to key a lock on, so the legacy path
/// stays uncoordinated, as it was before.
pub fn mutation_lock(journal: Option<&Path>) -> Result<Option<File>, String> {
    let Some(journal) = journal else {
        return Ok(None);
    };
    let path = journal.with_extension("lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|e| format!("open route helper lock {}: {e}", path.display()))?;
    // SAFETY: `file` is a valid open descriptor; flock reports every failure
    // through its return value and errno.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        return Err(format!(
            "another route helper operation holds the lock on {}: {error}",
            path.display()
        ));
    }
    Ok(Some(file))
}

pub struct Owner {
    path: PathBuf,
    session: Session<String>,
}

impl Owner {
    pub fn open(path: &Path, iface: &str, expected_session: &str) -> Result<Self, String> {
        if iface == RESERVATION_NAME {
            return Err("TUN interface name is reserved for ownership tracking".into());
        }
        let session: Session<String> = v2ray_rs_tun_session::read(path)
            .map_err(|e| format!("read TUN ownership journal: {e}"))?;
        if session.backend != "xray" || session.iface != iface {
            return Err(
                "journal backend or interface does not match the requested operation".into(),
            );
        }
        let ownership = session
            .ownership
            .as_ref()
            .ok_or("legacy TUN state requires explicit operator recovery")?;
        if ownership.session_id.to_string() != expected_session {
            return Err("TUN journal session identity changed".into());
        }
        ownership.verify_epoch().map_err(|e| e.to_string())?;
        Ok(Self {
            path: path.to_owned(),
            session,
        })
    }

    fn state(&self) -> &Ownership {
        self.session
            .ownership
            .as_ref()
            .expect("validated ownership")
    }

    fn state_mut(&mut self) -> &mut Ownership {
        self.session
            .ownership
            .as_mut()
            .expect("validated ownership")
    }

    fn save(&self) -> Result<(), String> {
        v2ray_rs_tun_session::write(&self.path, &self.session)
            .map_err(|e| format!("write TUN ownership journal: {e}"))
    }

    pub async fn reserve(&mut self, handle: &Handle) -> Result<(), String> {
        check_reserve_phase(self.state().phase)?;
        if let Some(link) = link(handle, RESERVATION_NAME).await? {
            self.verify_marker(&link)?;
            let live = self.verify_resources(handle).await?;
            self.verify_device(handle, true).await?;
            self.state_mut().resources = live;
            self.state_mut().pending = None;
        } else {
            if self.state().phase != Phase::Prepared
                || !self.state().resources.is_empty()
                || self.state().pending.is_some()
            {
                return Err(
                    "TUN kernel reservation is missing; explicit operator recovery is required"
                        .into(),
                );
            }
            if !snapshot(handle).await?.is_empty() {
                return Err("xray routing table or rule priorities are already in use".into());
            }
            if link(handle, &self.session.iface).await?.is_some() {
                return Err("TUN interface name is already in use".into());
            }
            handle
                .link()
                .add(
                    LinkDummy::new(RESERVATION_NAME)
                        .append_extra_attribute(LinkAttribute::IfAlias(self.state().alias()))
                        .build(),
                )
                .execute()
                .await
                .map_err(|e| format!("reserve xray routing state: {e}"))?;
            let marker = link(handle, RESERVATION_NAME)
                .await?
                .ok_or("TUN reservation disappeared")?;
            self.state_mut().reservation_ifindex = Some(marker.header.index);
            self.save()?;
            handle
                .link()
                .set(
                    LinkUnspec::new_with_index(marker.header.index)
                        .append_extra_attribute(LinkAttribute::IfAlias(self.state().alias()))
                        .build(),
                )
                .execute()
                .await
                .map_err(|e| format!("mark routing reservation ownership: {e}"))?;
            let marker = link(handle, RESERVATION_NAME)
                .await?
                .ok_or("TUN reservation disappeared")?;
            self.verify_marker(&marker)?;
        }
        self.state_mut().phase = Phase::Reserved;
        self.save()
    }

    fn verify_marker(&mut self, marker: &LinkMessage) -> Result<(), String> {
        let alias = self.state().alias();
        let dummy = marker.attributes.iter().any(|a| {
            matches!(a, LinkAttribute::LinkInfo(info)
            if info.contains(&LinkInfo::Kind(InfoKind::Dummy)))
        });
        if !dummy {
            return Err("routing reservation is not a dummy link".into());
        }
        if !marker.attributes.contains(&LinkAttribute::IfAlias(alias)) {
            return Err("routing reservation alias belongs to another session".into());
        }
        if self
            .state()
            .reservation_ifindex
            .is_some_and(|i| i != marker.header.index)
        {
            return Err("routing reservation interface index changed".into());
        }
        self.state_mut().reservation_ifindex = Some(marker.header.index);
        Ok(())
    }

    async fn verify_resources(&self, handle: &Handle) -> Result<Vec<Resource>, String> {
        let live = snapshot(handle).await?;
        verify_inventory(&self.state().resources, &live)?;
        Ok(live)
    }

    async fn verify_device(&self, handle: &Handle, absent_ok: bool) -> Result<Option<u32>, String> {
        let Some(device) = link(handle, &self.session.iface).await? else {
            return if absent_ok {
                Ok(None)
            } else {
                Err("TUN device is absent".into())
            };
        };
        if self.state().device_ifindex != Some(device.header.index)
            || !device
                .attributes
                .contains(&LinkAttribute::IfAlias(self.state().alias()))
            || !crate::validate::is_tun_device(&self.session.iface)
        {
            return Err("TUN device identity changed; refusing to touch networking".into());
        }
        Ok(Some(device.header.index))
    }

    pub async fn attach(&mut self, handle: &Handle, expected: u32) -> Result<(), String> {
        let marker = link(handle, RESERVATION_NAME)
            .await?
            .ok_or("TUN reservation is missing")?;
        self.verify_marker(&marker)?;
        self.verify_resources(handle).await?;
        let device = link(handle, &self.session.iface)
            .await?
            .ok_or("TUN device is absent")?;
        crate::validate::check_ifindex(&self.session.iface, expected, device.header.index)?;
        if device.attributes.iter().any(|a| {
            matches!(a, LinkAttribute::IfAlias(alias)
            if !alias.is_empty() && alias != &self.state().alias())
        }) {
            return Err("TUN device carries another owner's identity".into());
        }
        self.state_mut().device_ifindex = Some(expected);
        self.state_mut().phase = Phase::Applying;
        self.save()?;
        handle
            .link()
            .set(
                LinkUnspec::new_with_index(expected)
                    .append_extra_attribute(LinkAttribute::IfAlias(self.state().alias()))
                    .build(),
            )
            .execute()
            .await
            .map_err(|e| format!("mark TUN device ownership: {e}"))?;
        self.verify_device(handle, false).await?;
        Ok(())
    }

    pub async fn begin(&mut self, handle: &Handle, intent: Resource) -> Result<bool, String> {
        let live = self.verify_resources(handle).await?;
        let matching = live
            .iter()
            .filter(|resource| matches_intent(&intent, resource))
            .count();
        if matching > 1 {
            return Err("duplicate routing resources make ownership ambiguous".into());
        }
        if matching == 1 {
            return Ok(false);
        }
        self.state_mut().pending = Some(intent);
        self.save()?;
        Ok(true)
    }

    pub async fn finish(&mut self, handle: &Handle) -> Result<(), String> {
        let live = snapshot(handle).await?;
        let intent = self
            .state()
            .pending
            .as_ref()
            .ok_or("missing mutation intent")?;
        let matches: Vec<_> = live
            .iter()
            .filter(|resource| matches_intent(intent, resource))
            .collect();
        if matches.len() != 1 {
            return Err("routing mutation outcome is ambiguous; journal retained".into());
        }
        let observed = matches[0].clone();
        let mut owned = self.state().resources.clone();
        if !owned.contains(&observed) {
            owned.push(observed);
        }
        verify_inventory(&owned, &live)?;
        self.state_mut().resources = owned;
        self.state_mut().pending = None;
        self.save()
    }

    pub fn active(&mut self) -> Result<(), String> {
        check_active_phase(self.state().phase)?;
        self.state_mut().phase = Phase::Active;
        self.save()
    }

    pub async fn remove_exclusions(&mut self, handle: &Handle) -> Result<(), String> {
        self.remove_rules_at_priority(handle, net::RULE_PREF_EXCLUDE, None)
            .await
    }

    /// Removes owned rules at `priority` from the kernel and the journal,
    /// keeping only rules whose UID range is exactly `keep_uid` when one is
    /// given. Journal entries at `priority` whose rule is already absent from
    /// the kernel are pruned too, so an absent-flag rerun leaves neither a
    /// kernel rule nor a stale ownership record. Idempotent: an absent rule
    /// leaves the deletion satisfied, and a kept rule stays journaled.
    pub async fn remove_rules_at_priority(
        &mut self,
        handle: &Handle,
        priority: u32,
        keep_uid: Option<u32>,
    ) -> Result<(), String> {
        let live = self.verify_resources(handle).await?;
        for resource in live {
            if let Resource::Rule(bytes) = &resource {
                let rule = decode_rule(bytes)?;
                if rule_matches_removal(&rule, priority, keep_uid) {
                    remove(handle, &resource).await?;
                    self.state_mut()
                        .resources
                        .retain(|owned| owned != &resource);
                    self.save()?;
                }
            }
        }
        // The loop above removed every live rule matching the removal
        // predicate, so a journal entry still matching it claims kernel state
        // that no longer exists; dropping those keeps reruns converged instead
        // of accumulating unreachable ownership records.
        let mut stale = Vec::new();
        for owned in &self.state().resources {
            let Resource::Rule(bytes) = owned else {
                continue;
            };
            if rule_matches_removal(&decode_rule(bytes)?, priority, keep_uid) {
                stale.push(owned.clone());
            }
        }
        if !stale.is_empty() {
            self.state_mut()
                .resources
                .retain(|owned| !stale.contains(owned));
            self.save()?;
        }
        Ok(())
    }

    pub async fn cleanup(&mut self, handle: &Handle) -> Result<(), String> {
        if self.state().phase == Phase::Complete {
            return Ok(());
        }
        let Some(marker) = link(handle, RESERVATION_NAME).await? else {
            if self.state().phase == Phase::Prepared
                && self.state().resources.is_empty()
                && self.state().pending.is_none()
                && self.state().device_ifindex.is_none()
            {
                self.state_mut().phase = Phase::Complete;
                return self.save();
            }
            return Err("TUN reservation is missing; automatic cleanup is unsafe".into());
        };
        self.verify_marker(&marker)?;
        let live = self.verify_resources(handle).await?;
        let device = self.verify_device(handle, true).await?;
        self.state_mut().phase = Phase::Cleaning;
        self.save()?;
        let mut failures = Vec::new();
        for resource in &live {
            match remove(handle, resource).await {
                Err(error) => failures.push(error),
                Ok(()) => {
                    self.state_mut().resources.retain(|owned| owned != resource);
                    self.save()?;
                }
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("; "));
        }
        if !snapshot(handle).await?.is_empty() {
            return Err("routing state remains after cleanup".into());
        }
        if let Some(index) = device {
            self.verify_device(handle, true).await?;
            handle
                .link()
                .del(index)
                .execute()
                .await
                .map_err(|e| format!("delete owned TUN device: {e}"))?;
        }
        if link(handle, &self.session.iface).await?.is_some() {
            return Err("TUN device remains after cleanup".into());
        }
        let marker = link(handle, RESERVATION_NAME)
            .await?
            .ok_or("TUN reservation disappeared during cleanup")?;
        self.verify_marker(&marker)?;
        // The journal stays Cleaning if this acknowledgement is lost. Recovery
        // then refuses to assume a missing reservation proves successful cleanup.
        handle
            .link()
            .del(marker.header.index)
            .execute()
            .await
            .map_err(|e| format!("release TUN reservation: {e}"))?;
        self.state_mut().phase = Phase::Complete;
        self.state_mut().resources.clear();
        self.state_mut().pending = None;
        self.save()
    }
}

/// A crash respawn may resume from any pre-cleanup phase, but a journal left
/// in `Cleaning` by a failed teardown is not resumable.
fn check_reserve_phase(phase: Phase) -> Result<(), String> {
    match phase {
        Phase::Cleaning => Err(
            "interrupted cleanup is not resumable; explicit operator recovery is required".into(),
        ),
        Phase::Complete => Err("completed TUN journal cannot start another session".into()),
        Phase::Prepared | Phase::Reserved | Phase::Applying | Phase::Active => Ok(()),
    }
}

fn check_active_phase(phase: Phase) -> Result<(), String> {
    if phase == Phase::Applying {
        Ok(())
    } else {
        Err(format!(
            "TUN session phase is {phase:?}; only an applying session can be marked active"
        ))
    }
}

pub fn encode(message: &impl Emitable) -> Vec<u8> {
    let mut bytes = vec![0; message.buffer_len()];
    message.emit(&mut bytes);
    bytes
}

fn decode_rule(bytes: &[u8]) -> Result<RuleMessage, String> {
    RuleMessage::parse(&RuleMessageBuffer::new_checked(&bytes).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Whether a rule at `priority` is due for deletion: it matches the priority
/// and, when `keep_uid` is set, carries a different (or wider) UID range.
fn rule_matches_removal(rule: &RuleMessage, priority: u32, keep_uid: Option<u32>) -> bool {
    net::rule_priority(rule) == Some(priority)
        && keep_uid.is_none_or(|uid| net::rule_uid(rule) != Some(uid))
}

fn decode_route(bytes: &[u8]) -> Result<RouteMessage, String> {
    RouteMessage::parse(&RouteMessageBuffer::new_checked(&bytes).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn matches_intent(intent: &Resource, live: &Resource) -> bool {
    match (intent, live) {
        (Resource::Rule(a), Resource::Rule(b)) => {
            let (Ok(a), Ok(b)) = (decode_rule(a), decode_rule(b)) else {
                return false;
            };
            a.header.family == b.header.family
                && a.header.action == b.header.action
                && a.header.src_len == b.header.src_len
                && a.header.dst_len == b.header.dst_len
                && net::rule_table(&a) == net::rule_table(&b)
                && a.attributes
                    .iter()
                    .filter(|a| !matches!(a, RuleAttribute::Table(_)))
                    .all(|attribute| {
                        b.attributes.contains(attribute)
                            // Linux can omit a zero mark while still reporting its mask.
                            || (*attribute == RuleAttribute::FwMark(0)
                                && a.attributes.iter().any(
                                    |attr| matches!(attr, RuleAttribute::FwMask(mask) if *mask != 0),
                                )
                                && !b.attributes.iter().any(
                                    |attr| matches!(attr, RuleAttribute::FwMark(_)),
                                ))
                    })
        }
        (Resource::Route(a), Resource::Route(b)) => {
            let (Ok(a), Ok(b)) = (decode_route(a), decode_route(b)) else {
                return false;
            };
            a.header.address_family == b.header.address_family
                && a.header.destination_prefix_length == b.header.destination_prefix_length
                && a.header.kind == b.header.kind
                && (a.header.address_family == rtnetlink::packet_route::AddressFamily::Inet6
                    || a.header.scope == b.header.scope)
                && a.header.protocol == b.header.protocol
                && route_table(&a) == route_table(&b)
                && a.attributes
                    .iter()
                    .filter(|attribute| {
                        !(matches!(attribute, RouteAttribute::Table(_))
                            || a.header.destination_prefix_length == 0
                                && matches!(attribute, RouteAttribute::Destination(_)))
                    })
                    .all(|attribute| b.attributes.contains(attribute))
        }
        _ => false,
    }
}

pub fn verify_inventory(owned: &[Resource], live: &[Resource]) -> Result<(), String> {
    let mut matched = vec![false; owned.len()];
    for resource in live {
        let Some(index) = owned
            .iter()
            .enumerate()
            .find_map(|(index, owned)| (!matched[index] && owned == resource).then_some(index))
        else {
            return Err(
                "foreign or unacknowledged routing state found. Automatic mutation refused".into(),
            );
        };
        matched[index] = true;
    }
    Ok(())
}

async fn remove(handle: &Handle, resource: &Resource) -> Result<(), String> {
    let result = match resource {
        Resource::Rule(bytes) => handle.rule().del(decode_rule(bytes)?).execute().await,
        Resource::Route(bytes) => handle.route().del(decode_route(bytes)?).execute().await,
    };
    match result {
        Ok(()) => Ok(()),
        // The kernel may have dropped the object between the snapshot and this
        // delete; that outcome leaves nothing to remove.
        Err(e) if net::is_no_such_object(&e) => Ok(()),
        Err(e) => Err(format!("delete journaled resource: {e}")),
    }
}

fn route_table(route: &RouteMessage) -> u32 {
    route
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            RouteAttribute::Table(table) => Some(*table),
            _ => None,
        })
        .unwrap_or(route.header.table as u32)
}

async fn snapshot(handle: &Handle) -> Result<Vec<Resource>, String> {
    let mut resources = Vec::new();
    for version in [IpVersion::V4, IpVersion::V6] {
        if version == IpVersion::V6 && !net::host_has_ipv6() {
            continue;
        }
        let mut rules = handle.rule().get(version).execute();
        while let Some(rule) = rules
            .try_next()
            .await
            .map_err(|e| format!("read routing rules: {e}"))?
        {
            if net::is_xray_rule(&rule) || net::rule_table(&rule) == net::XRAY_ROUTE_TABLE {
                resources.push(Resource::Rule(encode(&rule)));
            }
        }
    }
    let v4 = RouteMessageBuilder::<std::net::Ipv4Addr>::new().build();
    let v6 = RouteMessageBuilder::<std::net::Ipv6Addr>::new().build();
    for query in [v4, v6] {
        if query.header.address_family == rtnetlink::packet_route::AddressFamily::Inet6
            && !net::host_has_ipv6()
        {
            continue;
        }
        let mut routes = handle.route().get(query).execute();
        while let Some(route) = routes
            .try_next()
            .await
            .map_err(|e| format!("read routes: {e}"))?
        {
            if route_table(&route) == net::XRAY_ROUTE_TABLE {
                resources.push(Resource::Route(encode(&route)));
            }
        }
    }
    Ok(resources)
}

async fn link(handle: &Handle, name: &str) -> Result<Option<LinkMessage>, String> {
    let mut links = handle.link().get().match_name(name.to_owned()).execute();
    match links.try_next().await {
        Ok(link) => Ok(link),
        Err(error) if net::is_no_such_device(&error) => Ok(None),
        Err(error) => Err(format!("read link {name}: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_refuses_foreign_and_duplicate_objects() {
        let own = Resource::Rule(vec![0]);
        let foreign = Resource::Rule(vec![1]);
        assert!(
            verify_inventory(std::slice::from_ref(&own), std::slice::from_ref(&foreign)).is_err()
        );
        assert!(verify_inventory(std::slice::from_ref(&own), &[own.clone(), own.clone()]).is_err());
        assert!(verify_inventory(&[own], &[]).is_ok());
    }

    #[test]
    fn journal_identity_and_epoch_are_required_before_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let mut session = Session {
            backend: "xray".to_string(),
            iface: "tun0".to_string(),
            ownership: Some(Ownership::new().unwrap()),
        };
        let id = session.ownership.as_ref().unwrap().session_id.to_string();
        v2ray_rs_tun_session::write(&path, &session).unwrap();
        assert!(Owner::open(&path, "tun0", &id).is_ok());
        assert!(Owner::open(&path, "tun0", "").is_err());
        assert!(Owner::open(&path, "tun1", &id).is_err());
        session.ownership.as_mut().unwrap().epoch.namespace_inode += 1;
        v2ray_rs_tun_session::write(&path, &session).unwrap();
        assert!(Owner::open(&path, "tun0", &id).is_err());
        session.ownership = None;
        v2ray_rs_tun_session::write(&path, &session).unwrap();
        assert!(Owner::open(&path, "tun0", &id).is_err());
    }

    #[test]
    fn acknowledged_rules_are_compared_with_complete_selectors() {
        let mut first = RuleMessage::default();
        first.attributes.push(RuleAttribute::Priority(8997));
        let mut second = first.clone();
        second.attributes.push(RuleAttribute::Table(254));
        let a = Resource::Rule(encode(&first));
        let b = Resource::Rule(encode(&second));
        assert!(verify_inventory(&[a], &[b]).is_err());
    }

    #[test]
    fn rule_intent_accepts_omitted_zero_mark_only_with_matching_mask() {
        let mut requested = RuleMessage::default();
        requested.attributes = vec![RuleAttribute::FwMark(0), RuleAttribute::FwMask(255)];
        let intent = Resource::Rule(encode(&requested));

        let mut reported = requested;
        reported.attributes = vec![RuleAttribute::FwMask(255)];
        assert!(matches_intent(&intent, &Resource::Rule(encode(&reported))));

        reported.attributes.push(RuleAttribute::FwMark(1));
        assert!(!matches_intent(&intent, &Resource::Rule(encode(&reported))));

        reported.attributes = vec![RuleAttribute::FwMask(127)];
        assert!(!matches_intent(&intent, &Resource::Rule(encode(&reported))));

        reported.attributes.clear();
        assert!(!matches_intent(&intent, &Resource::Rule(encode(&reported))));
    }

    #[test]
    fn marker_without_owned_alias_is_refused_without_journal_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let session = Session {
            backend: "xray".to_string(),
            iface: "tun0".to_string(),
            ownership: Some(Ownership::new().unwrap()),
        };
        let id = session.ownership.as_ref().unwrap().session_id.to_string();
        v2ray_rs_tun_session::write(&path, &session).unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut owner = Owner::open(&path, "tun0", &id).unwrap();

        let marker = |alias: Option<&str>| {
            let mut message = LinkMessage::default();
            message.header.index = 7;
            message
                .attributes
                .push(LinkAttribute::LinkInfo(vec![LinkInfo::Kind(
                    InfoKind::Dummy,
                )]));
            if let Some(alias) = alias {
                message
                    .attributes
                    .push(LinkAttribute::IfAlias(alias.to_owned()));
            }
            message
        };

        // Interrupted tagging: the dummy link carries no alias at all.
        assert!(owner.verify_marker(&marker(None)).is_err());
        // A live alias from another session is equally refused.
        assert!(
            owner
                .verify_marker(&marker(Some(
                    "v2ray-rs:00000000-0000-0000-0000-000000000000"
                )))
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn inventory_accepts_a_live_subset_of_owned() {
        let kept = Resource::Rule(vec![0]);
        let removed = Resource::Rule(vec![1]);
        let owned = [kept.clone(), removed];
        let live = std::slice::from_ref(&kept);
        assert!(verify_inventory(&owned, live).is_ok());

        let live = [kept.clone(), kept.clone()];
        let owned = std::slice::from_ref(&kept);
        assert!(verify_inventory(owned, &live).is_err());
    }

    #[test]
    fn phase_gates_allow_only_documented_transitions() {
        assert!(check_active_phase(Phase::Applying).is_ok());
        for phase in [
            Phase::Prepared,
            Phase::Reserved,
            Phase::Active,
            Phase::Cleaning,
            Phase::Complete,
        ] {
            assert!(check_active_phase(phase).is_err(), "{phase:?}");
        }
        for phase in [
            Phase::Prepared,
            Phase::Reserved,
            Phase::Applying,
            Phase::Active,
        ] {
            assert!(check_reserve_phase(phase).is_ok(), "{phase:?}");
        }
        assert!(check_reserve_phase(Phase::Cleaning).is_err());
        assert!(check_reserve_phase(Phase::Complete).is_err());
    }
}
