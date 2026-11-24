use super::*;

/// Runs a full stop and parks the manager only when its cleanup failed: a
/// parked manager still owns its rules, so dropping it would leave them
/// installed with no owner to retry the removal.
pub(super) async fn stop_or_park(
    mut mgr: ProcessManager,
    reason: StopReason,
    pending: &mut Vec<ProcessManager>,
) {
    if mgr.shutdown_with(reason).await.is_err() {
        pending.push(mgr);
    }
}

/// Exit tail shared by every stop and host-error exit. The orphaned
/// managers join `pending` before the emptiness check: a superseded
/// manager whose cleanup failed still owns kernel state, so it holds the
/// session in `retry_cleanup` instead of being dropped with its rules
/// installed. `on_clean` is reported only once nothing is left to clean.
pub(super) async fn finish_or_retry_cleanup(
    pending: Vec<ProcessManager>,
    orphaned: &mut Vec<ProcessManager>,
    generation: u64,
    cmd_rx: &mut mpsc::UnboundedReceiver<SessionCmd>,
    sender: &EventSender,
    on_clean: ProcessState,
) {
    let mut pending = pending;
    pending.append(orphaned);
    if pending.is_empty() {
        sender.emit(SessionEvent::State(generation, on_clean, None));
    } else if pending
        .iter()
        .any(ProcessManager::requires_operator_recovery)
    {
        require_recovery(pending, generation, cmd_rx, sender).await;
    } else {
        retry_cleanup(pending, generation, cmd_rx, sender, on_clean).await;
    }
}

/// Releases the manager an earlier candidate parked once `mgr` replaces
/// it. A replacement that adopted the parked manager's TUN identity owns
/// that kernel state, so the parked runtime is disarmed and only its
/// process, readers, and engine are released. Any other replacement gets
/// a full stop; one that fails keeps the manager in `orphaned` for a retry.
pub(super) async fn supersede_parked(
    parked: &mut Option<ProcessManager>,
    mgr: &ProcessManager,
    orphaned: &mut Vec<ProcessManager>,
) {
    let Some(mut stale) = parked.take() else {
        return;
    };
    let adopted = mgr.tun_identity().is_some() && mgr.tun_identity() == stale.tun_identity();
    if adopted {
        stale.abandon_superseded_tun();
    }
    stop_or_park(stale, StopReason::NodeSwitch, orphaned).await;
}

/// The message for parked cleanups: one entry per manager, so the UI shows
/// everything that may remain installed, not just the first failure.
fn cleanup_message(pending: &[ProcessManager]) -> String {
    pending
        .iter()
        .map(|mgr| match mgr.state() {
            ProcessState::Error(text) => text.clone(),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Holds managers whose cleanup failed: no terminal state is reported while
/// their rules may remain. The next `Stop` retries every parked cleanup, and
/// only when all succeed is the terminal `on_clean` state emitted — a
/// candidate failure reports its own terminal error here, never a clean
/// disconnect. A closed channel has no one left to retry, so the retained
/// failure is reported as terminal.
pub(super) async fn retry_cleanup(
    pending: Vec<ProcessManager>,
    generation: u64,
    cmd_rx: &mut mpsc::UnboundedReceiver<SessionCmd>,
    sender: &EventSender,
    on_clean: ProcessState,
) {
    let mut pending = pending;
    sender.emit(SessionEvent::CleanupPending(
        generation,
        cleanup_message(&pending),
    ));
    loop {
        match cmd_rx.recv().await {
            Some(SessionCmd::Stop(_)) => {
                let mut remaining = Vec::new();
                for mut mgr in pending.drain(..) {
                    if mgr.stop().await.is_err() {
                        remaining.push(mgr);
                    }
                }
                if remaining.is_empty() {
                    sender.emit(SessionEvent::State(generation, on_clean, None));
                    return;
                }
                pending = remaining;
                sender.emit(SessionEvent::CleanupPending(
                    generation,
                    cleanup_message(&pending),
                ));
            }
            None => {
                // Closed channel: nobody can retry, so report every parked
                // failure once as the terminal state.
                sender.emit(SessionEvent::State(
                    generation,
                    ProcessState::Error(cleanup_message(&pending)),
                    None,
                ));
                return;
            }
        }
    }
}

/// Holds managers whose cleanup cannot proceed: the engine exit is
/// unverified, so neither a retry nor a replacement may touch the host. The
/// recovery state stays presented until the channel closes; a further Stop
/// only re-presents it, never re-enqueues an attempt.
pub(super) async fn require_recovery(
    pending: Vec<ProcessManager>,
    generation: u64,
    cmd_rx: &mut mpsc::UnboundedReceiver<SessionCmd>,
    sender: &EventSender,
) {
    sender.emit(SessionEvent::CleanupRecoveryRequired(
        generation,
        cleanup_message(&pending),
    ));
    loop {
        match cmd_rx.recv().await {
            Some(SessionCmd::Stop(_)) => {
                sender.emit(SessionEvent::CleanupRecoveryRequired(
                    generation,
                    cleanup_message(&pending),
                ));
            }
            None => {
                // Closed channel: nobody can observe anymore, so report the
                // retained failure once as the terminal state.
                sender.emit(SessionEvent::State(
                    generation,
                    ProcessState::Error(cleanup_message(&pending)),
                    None,
                ));
                return;
            }
        }
    }
}

/// A candidate's failure kept split: the summary prints `label: reason`, while
/// `key` is the normalized form the loop compares to spot a failure that every
/// candidate shares and that failing over cannot fix.
pub(super) struct CandidateFailure {
    label: String,
    reason: String,
    key: String,
}

impl CandidateFailure {
    pub(super) fn new(label: &str, reason: &str, address: &str, port: u16) -> Self {
        let reason = strip_ansi(reason).into_owned();
        Self {
            key: failure_key(&reason, label, address, port),
            label: label.to_string(),
            reason,
        }
    }
}

pub(super) fn record_failure(
    failures: &mut Vec<CandidateFailure>,
    failure: CandidateFailure,
    position: usize,
    total: usize,
) {
    log::info!(
        "candidate failed {position}/{total} label={} reason={}",
        failure.label,
        failure.reason
    );
    failures.push(failure);
}

/// Classifies a failed candidate from the manager's captured backend output.
/// The manager drains its log readers before reporting a failed start or a
/// crash give-up, so the buffer holds every line the backend printed by the
/// time the candidate is marked failed.
pub(super) fn classify_candidate(
    mgr: &ProcessManager,
    backend: BackendType,
) -> Option<FailureClassification> {
    let buffer = mgr
        .log_buffer()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let captured = buffer
        .last_n(buffer.len())
        .iter()
        .map(|l| l.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    classify_failure(backend, &captured)
}

/// Offers a subscription refresh for a classified failure, at most once per
/// connection per subscription: failover across several nodes of the same
/// subscription must not stack repeated offers. The refresh itself stays
/// user-initiated — nothing here starts an update.
pub(super) fn emit_refresh_offer(
    sender: &EventSender,
    subscriptions: &[Subscription],
    offered: &mut HashSet<uuid::Uuid>,
    subscription_id: uuid::Uuid,
    label: &str,
    cause: &str,
) {
    if !offered.insert(subscription_id) {
        return;
    }
    let name = subscriptions
        .iter()
        .find(|s| s.id == subscription_id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "subscription".to_string());
    sender.emit(SessionEvent::OfferSubscriptionRefresh {
        subscription_id,
        message: format!("{label} ({name}): {cause}"),
    });
}

/// Whether the last two candidates failed the same way.
pub(super) fn repeats_previous(failures: &[CandidateFailure]) -> bool {
    match failures {
        [.., previous, last] => previous.key == last.key,
        _ => false,
    }
}

/// The comparable form of a failure: everything that varies between candidates
/// purely because they are different attempts — colors, log timestamps, message
/// counters, the node's own name, address and port — collapses to a placeholder,
/// so two texts compare equal exactly when they describe the same failure.
pub(super) fn failure_key(reason: &str, label: &str, address: &str, port: u16) -> String {
    let stripped = strip_ansi(reason);
    let mut key = mask_counters(strip_leading_timestamp(stripped.trim()));
    for token in [label, address] {
        // A two-letter remark such as "US" would mask unrelated substrings.
        if token.chars().count() >= 3 {
            key = key.replace(token, "<node>");
        }
    }
    mask_port(&key, port)
}

/// Removes one leading `YYYY/MM/DD HH:MM:SS(.frac)` stamp. Only the leading one
/// is an artifact of when the attempt ran; a date inside the text is content.
pub(crate) fn strip_leading_timestamp(text: &str) -> &str {
    let b = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        range
            .clone()
            .all(|i| b.get(i).is_some_and(u8::is_ascii_digit))
    };
    let literal = |i: usize, c: u8| b.get(i) == Some(&c);
    let shaped = digits(0..4)
        && literal(4, b'/')
        && digits(5..7)
        && literal(7, b'/')
        && digits(8..10)
        && literal(10, b' ')
        && digits(11..13)
        && literal(13, b':')
        && digits(14..16)
        && literal(16, b':')
        && digits(17..19);
    if !shaped {
        return text;
    }

    let mut end = 19;
    if literal(19, b'.') {
        let fraction = b
            .get(20..)
            .unwrap_or_default()
            .iter()
            .take(9)
            .take_while(|c| c.is_ascii_digit())
            .count();
        if fraction > 0 {
            end = 20 + fraction;
        }
    }
    while literal(end, b' ') {
        end += 1;
    }
    &text[end..]
}

/// Collapses bracketed all-digit tokens such as sing-box's `[0000]` message
/// counter. Named brackets like `[tun-in]` are content and stay.
fn mask_counters(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find(']') {
            Some(close) if close > 0 && after[..close].bytes().all(|c| c.is_ascii_digit()) => {
                out.push_str("[N]");
                rest = &after[close + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Replaces the candidate's own port where the text reads it as a port: right
/// after a `:` and not running into another number. Requiring the colon keeps a
/// short port such as `80` out of prose like `80% packet loss`, and the trailing
/// check keeps `443` from being clipped out of `14430` or `10.4.43.1`.
fn mask_port(text: &str, port: u16) -> String {
    let needle = port.to_string();
    let bytes = text.as_bytes();
    let free = |c: Option<u8>| !c.is_some_and(|c| c.is_ascii_digit() || c == b'.');
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with(&needle)
            && i.checked_sub(1).map(|j| bytes[j]) == Some(b':')
            && free(bytes.get(i + needle.len()).copied())
        {
            out.push_str("<port>");
            i += needle.len();
            continue;
        }
        let c = text[i..].chars().next().expect("index on a char boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// The message for the single terminal state. Two candidates that failed the
/// same way say nothing about any node, so a per-node list would only invite
/// another pointless retry.
pub(super) fn terminal_failure(failures: &[CandidateFailure]) -> String {
    match failures.last() {
        Some(last) if repeats_previous(failures) => format!(
            "Connection failed on consecutive nodes with the same error (not node-specific): {}",
            last.reason
        ),
        _ => summarize_failures(failures),
    }
}

pub(super) fn summarize_failures(failures: &[CandidateFailure]) -> String {
    if failures.is_empty() {
        return "All candidates failed".into();
    }

    let preview = failures
        .iter()
        .take(3)
        .map(|f| format!("{}: {}", f.label, f.reason))
        .collect::<Vec<_>>()
        .join("; ");

    if failures.len() > 3 {
        format!("All candidates failed: {preview}; ...")
    } else {
        format!("All candidates failed: {preview}")
    }
}
