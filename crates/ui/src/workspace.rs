use std::sync::{Arc, RwLock, mpsc};

use tokio::sync::oneshot;
use v2ray_rs_core::models::{AppSettings, ManualNode, RoutingRuleSet, Subscription};
use v2ray_rs_core::persistence::{self, AppPaths, PersistenceError};
use v2ray_rs_core::resolve::LatencySnapshot;

type TransactionResult<T> = Result<T, PersistenceError>;

/// Body of one queued transaction: read the committed snapshot, apply the
/// mutation, persist it, publish the result, and answer the receipt.
type JobFn<T> = dyn FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<T> + Send;

/// Delivery of one transaction result to its waiting receipt.
type ReplyFn<T> = dyn FnOnce(TransactionResult<T>) + Send;

/// Receipt for a transaction queued on the workspace writer.
///
/// The writer publishes the committed snapshot before it resolves the receipt.
/// A transaction that reaches the writer always answers through the inner
/// result, including a submission that finds the writer already closed. The
/// outer error only appears when the writer thread stopped without answering,
/// for example after a panic.
pub type PersistenceReceipt<T> = oneshot::Receiver<TransactionResult<T>>;

fn writer_stopped() -> PersistenceError {
    PersistenceError::CorruptConfig("workspace writer stopped".into())
}

/// Blocks the current thread until a queued receipt resolves.
///
/// Startup warming runs before the GTK loop and can run with a Tokio runtime
/// already entered (the joined GTK tests), where
/// `oneshot::Receiver::blocking_recv` refuses to block. Poll the same receipt
/// with a thread waker instead of opening a second reply channel.
fn wait_for_receipt<T>(receipt: PersistenceReceipt<T>) -> TransactionResult<T> {
    use std::task::{Context, Poll, Wake, Waker};

    struct ThreadWake(std::thread::Thread);

    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut receipt = Box::pin(receipt);
    loop {
        match receipt.as_mut().poll(&mut context) {
            Poll::Ready(Ok(result)) => return result,
            Poll::Ready(Err(_)) => return Err(writer_stopped()),
            Poll::Pending => std::thread::park(),
        }
    }
}

trait Transaction: Send {
    fn run(self: Box<Self>, paths: &AppPaths, snapshot: &RwLock<Snapshot>);
    fn fail(self: Box<Self>, error: PersistenceError);
}

struct Pending<T> {
    run: Box<JobFn<T>>,
    reply: Box<ReplyFn<T>>,
}

impl<T: Send + 'static> Transaction for Pending<T> {
    fn run(self: Box<Self>, paths: &AppPaths, snapshot: &RwLock<Snapshot>) {
        let Pending { run, reply } = *self;
        reply(run(paths, snapshot));
    }

    fn fail(self: Box<Self>, error: PersistenceError) {
        let Pending { reply, .. } = *self;
        reply(Err(error));
    }
}

/// Test-only gate that holds the next queued transaction before it runs.
///
/// The GTK recovery tests reuse it to prove the loop stays responsive while an
/// ordered write is in flight: arm it, queue a transaction, observe
/// `is_parked`, then release.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct WriterLatch {
    state: std::sync::Mutex<LatchState>,
    changed: std::sync::Condvar,
}

#[cfg(test)]
#[derive(Default)]
struct LatchState {
    armed: bool,
    parked: bool,
    released: bool,
}

#[cfg(test)]
impl WriterLatch {
    pub(crate) fn arm(&self) {
        let mut state = self.lock();
        state.armed = true;
        state.parked = false;
        state.released = false;
    }

    /// Blocks the writer thread before it executes a job while armed.
    fn wait_before_job(&self) {
        let mut state = self.lock();
        if !state.armed {
            return;
        }
        state.parked = true;
        self.changed.notify_all();
        while !state.released {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        state.armed = false;
        state.parked = false;
        self.changed.notify_all();
    }

    /// Blocks the test until the writer actually reaches the latch.
    pub(crate) fn wait_until_parked(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut state = self.lock();
        while !state.parked {
            let now = std::time::Instant::now();
            assert!(
                now < deadline,
                "workspace writer did not reach the test latch"
            );
            state = self
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }

    pub(crate) fn is_parked(&self) -> bool {
        self.lock().parked
    }

    /// Releases the parked job and waits until the writer leaves the latch, so
    /// consecutive arm/release cycles are deterministic.
    pub(crate) fn release(&self) {
        let mut state = self.lock();
        state.released = true;
        self.changed.notify_all();
        while state.parked {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LatchState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

/// Serializes every workspace write on one background thread.
///
/// A job is a whole transaction: it reads the latest committed snapshot,
/// applies the mutation, validates and persists the result, and only then
/// publishes the new snapshot and answers. Jobs run in submission order, so a
/// successful receipt means the bytes reached disk before dependent actions
/// run.
struct Writer {
    tx: mpsc::Sender<Box<dyn Transaction>>,
    #[cfg(test)]
    latch: Arc<WriterLatch>,
}

impl Writer {
    fn new(paths: AppPaths, snapshot: Arc<RwLock<Snapshot>>) -> Self {
        let (tx, rx) = mpsc::channel::<Box<dyn Transaction>>();
        #[cfg(test)]
        let latch = Arc::new(WriterLatch::default());
        #[cfg(test)]
        let thread_latch = latch.clone();
        std::thread::Builder::new()
            .name("workspace-writer".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    #[cfg(test)]
                    thread_latch.wait_before_job();
                    job.run(&paths, &snapshot);
                }
            })
            .expect("spawn workspace writer");
        Self {
            tx,
            #[cfg(test)]
            latch,
        }
    }

    /// Enqueues without waiting. A closed channel fails the transaction through
    /// its own reply, so the caller sees an explicit error instead of silence.
    fn submit<T, F, R>(&self, run: F, reply: R)
    where
        T: Send + 'static,
        F: FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<T> + Send + 'static,
        R: FnOnce(TransactionResult<T>) + Send + 'static,
    {
        let pending: Box<dyn Transaction> = Box::new(Pending {
            run: Box::new(run),
            reply: Box::new(reply),
        });
        if let Err(error) = self.tx.send(pending) {
            error.0.fail(writer_stopped());
        }
    }

    fn enqueue<T, F>(&self, run: F) -> PersistenceReceipt<T>
    where
        T: Send + 'static,
        F: FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<T> + Send + 'static,
    {
        let (reply, receipt) = oneshot::channel();
        self.submit(run, move |result| {
            let _ = reply.send(result);
        });
        receipt
    }
}

/// Cached outcome of loading one persisted store.
///
/// A failure is cached so a warmed snapshot never reads the disk again for a
/// corrupt or unreadable store, and never substitutes defaults for it. Only
/// the warm job caches failures: a cold fallback load that fails leaves the
/// slot empty so a caller can retry after the file is repaired.
#[derive(Clone)]
enum Cached<T> {
    Loaded(Arc<T>),
    Failed(String),
}

fn cached_result<T>(cached: Cached<T>) -> TransactionResult<Arc<T>> {
    match cached {
        Cached::Loaded(value) => Ok(value),
        Cached::Failed(message) => Err(cached_load_error(message)),
    }
}

/// Reports a cached warm failure with its original text. The failed load is
/// never turned into defaults; only the error variant is reconstructed because
/// the snapshot keeps the message, not the error value.
fn cached_load_error(message: String) -> PersistenceError {
    PersistenceError::CorruptConfig(message)
}

#[derive(Clone, Default)]
struct Snapshot {
    settings: Option<Cached<AppSettings>>,
    subscriptions: Option<Cached<Vec<Subscription>>>,
    manual_nodes: Option<Cached<Vec<ManualNode>>>,
    routing_rules: Option<Cached<RoutingRuleSet>>,
    latency: Option<Cached<LatencySnapshot>>,
}

fn with_snapshot<T>(snapshot: &RwLock<Snapshot>, read: impl FnOnce(&Snapshot) -> T) -> T {
    read(&snapshot.read().unwrap_or_else(|error| error.into_inner()))
}

fn publish(snapshot: &RwLock<Snapshot>, mutate: impl FnOnce(&mut Snapshot)) {
    mutate(&mut snapshot.write().unwrap_or_else(|error| error.into_inner()));
}

/// Installs a value loaded by a reader only when no writer or warm job
/// published an outcome in the meantime. Readers never hold the lock across
/// disk I/O.
fn publish_if_absent<T>(
    snapshot: &RwLock<Snapshot>,
    loaded: Arc<T>,
    slot: fn(&mut Snapshot) -> &mut Option<Cached<T>>,
) -> TransactionResult<Arc<T>> {
    let mut state = snapshot.write().unwrap_or_else(|error| error.into_inner());
    match slot(&mut state) {
        Some(Cached::Loaded(existing)) => Ok(existing.clone()),
        Some(Cached::Failed(message)) => Err(cached_load_error(message.clone())),
        slot @ None => {
            *slot = Some(Cached::Loaded(loaded.clone()));
            Ok(loaded)
        }
    }
}

/// Caches a warm load failure so later readers never read the disk again. A
/// value published while the load was running wins over the failure.
fn cache_failure<T>(
    snapshot: &RwLock<Snapshot>,
    message: String,
    slot: fn(&mut Snapshot) -> &mut Option<Cached<T>>,
) {
    let mut state = snapshot.write().unwrap_or_else(|error| error.into_inner());
    let slot = slot(&mut state);
    if slot.is_none() {
        *slot = Some(Cached::Failed(message));
    }
}

fn warm_store<T>(
    snapshot: &RwLock<Snapshot>,
    cached: fn(&Snapshot) -> Option<Cached<T>>,
    slot: fn(&mut Snapshot) -> &mut Option<Cached<T>>,
    load: impl FnOnce() -> TransactionResult<T>,
) -> TransactionResult<Arc<T>> {
    if let Some(cached) = with_snapshot(snapshot, cached) {
        return cached_result(cached);
    }
    match load() {
        Ok(value) => publish_if_absent(snapshot, Arc::new(value), slot),
        Err(error) => {
            cache_failure(snapshot, error.to_string(), slot);
            Err(error)
        }
    }
}

fn committed_settings(
    snapshot: &RwLock<Snapshot>,
    paths: &AppPaths,
) -> TransactionResult<Arc<AppSettings>> {
    match with_snapshot(snapshot, |state| state.settings.clone()) {
        Some(cached) => cached_result(cached),
        None => Ok(Arc::new(persistence::load_settings(paths)?)),
    }
}

fn committed_subscriptions(
    snapshot: &RwLock<Snapshot>,
    paths: &AppPaths,
) -> TransactionResult<Arc<Vec<Subscription>>> {
    match with_snapshot(snapshot, |state| state.subscriptions.clone()) {
        Some(cached) => cached_result(cached),
        None => Ok(Arc::new(persistence::load_subscriptions(paths)?)),
    }
}

fn committed_manual_nodes(
    snapshot: &RwLock<Snapshot>,
    paths: &AppPaths,
) -> TransactionResult<Arc<Vec<ManualNode>>> {
    match with_snapshot(snapshot, |state| state.manual_nodes.clone()) {
        Some(cached) => cached_result(cached),
        None => Ok(Arc::new(persistence::load_manual_nodes(paths)?)),
    }
}

fn committed_latency(
    snapshot: &RwLock<Snapshot>,
    paths: &AppPaths,
) -> TransactionResult<Arc<LatencySnapshot>> {
    match with_snapshot(snapshot, |state| state.latency.clone()) {
        Some(cached) => cached_result(cached),
        None => Ok(Arc::new(persistence::load_latency_snapshot(paths)?)),
    }
}

fn save_settings_job(
    settings: AppSettings,
) -> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<Arc<AppSettings>> + Send + 'static
{
    move |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        persistence::save_settings(paths, &settings)?;
        let committed = Arc::new(settings);
        publish(snapshot, |state| {
            state.settings = Some(Cached::Loaded(committed.clone()))
        });
        Ok(committed)
    }
}

fn update_settings_job<F, R>(
    mutate: F,
) -> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<(R, Arc<AppSettings>)> + Send + 'static
where
    F: FnOnce(&mut AppSettings) -> R + Send + 'static,
    R: Send + 'static,
{
    move |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        let mut settings = (*committed_settings(snapshot, paths)?).clone();
        let output = mutate(&mut settings);
        persistence::save_settings(paths, &settings)?;
        let committed = Arc::new(settings);
        publish(snapshot, |state| {
            state.settings = Some(Cached::Loaded(committed.clone()))
        });
        Ok((output, committed))
    }
}

#[cfg(test)]
fn save_subscriptions_job(
    subscriptions: Vec<Subscription>,
) -> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<Arc<Vec<Subscription>>>
+ Send
+ 'static {
    move |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        persistence::save_subscriptions(paths, &subscriptions)?;
        let committed = Arc::new(subscriptions);
        publish(snapshot, |state| {
            state.subscriptions = Some(Cached::Loaded(committed.clone()))
        });
        Ok(committed)
    }
}

type SharedSubscriptions = Arc<Vec<Subscription>>;
type SubscriptionsUpdate<R> = TransactionResult<(R, SharedSubscriptions)>;

fn update_subscriptions_job<F, R>(
    mutate: F,
) -> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> SubscriptionsUpdate<R> + Send + 'static
where
    F: FnOnce(&mut Vec<Subscription>) -> R + Send + 'static,
    R: Send + 'static,
{
    move |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        let mut subscriptions = (*committed_subscriptions(snapshot, paths)?).clone();
        let output = mutate(&mut subscriptions);
        persistence::save_subscriptions(paths, &subscriptions)?;
        let committed = Arc::new(subscriptions);
        publish(snapshot, |state| {
            state.subscriptions = Some(Cached::Loaded(committed.clone()))
        });
        Ok((output, committed))
    }
}

#[cfg(test)]
fn save_manual_nodes_job(
    nodes: Vec<ManualNode>,
) -> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<Arc<Vec<ManualNode>>> + Send + 'static
{
    move |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        persistence::save_manual_nodes(paths, &nodes)?;
        let committed = Arc::new(nodes);
        publish(snapshot, |state| {
            state.manual_nodes = Some(Cached::Loaded(committed.clone()))
        });
        Ok(committed)
    }
}

type SharedManualNodes = Arc<Vec<ManualNode>>;
type ManualNodesUpdate<R> = TransactionResult<(R, SharedManualNodes)>;

fn update_manual_nodes_job<F, R>(
    mutate: F,
) -> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> ManualNodesUpdate<R> + Send + 'static
where
    F: FnOnce(&mut Vec<ManualNode>) -> R + Send + 'static,
    R: Send + 'static,
{
    move |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        let mut nodes = (*committed_manual_nodes(snapshot, paths)?).clone();
        let output = mutate(&mut nodes);
        persistence::save_manual_nodes(paths, &nodes)?;
        let committed = Arc::new(nodes);
        publish(snapshot, |state| {
            state.manual_nodes = Some(Cached::Loaded(committed.clone()))
        });
        Ok((output, committed))
    }
}

fn save_routing_rules_job(
    rules: RoutingRuleSet,
) -> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<Arc<RoutingRuleSet>> + Send + 'static
{
    move |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        persistence::save_routing_rules(paths, &rules)?;
        let committed = Arc::new(rules);
        publish(snapshot, |state| {
            state.routing_rules = Some(Cached::Loaded(committed.clone()))
        });
        Ok(committed)
    }
}

#[cfg(test)]
fn save_latency_job(
    latency: LatencySnapshot,
) -> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<Arc<LatencySnapshot>> + Send + 'static
{
    move |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        persistence::save_latency_snapshot(paths, &latency)?;
        let committed = Arc::new(latency);
        publish(snapshot, |state| {
            state.latency = Some(Cached::Loaded(committed.clone()))
        });
        Ok(committed)
    }
}

fn update_latency_job<F, R>(
    mutate: F,
) -> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<(R, Arc<LatencySnapshot>)>
+ Send
+ 'static
where
    F: FnOnce(&mut LatencySnapshot) -> R + Send + 'static,
    R: Send + 'static,
{
    move |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        let mut latency = (*committed_latency(snapshot, paths)?).clone();
        let output = mutate(&mut latency);
        persistence::save_latency_snapshot(paths, &latency)?;
        let committed = Arc::new(latency);
        publish(snapshot, |state| {
            state.latency = Some(Cached::Loaded(committed.clone()))
        });
        Ok((output, committed))
    }
}

fn reset_settings_job()
-> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<Arc<AppSettings>> + Send + 'static
{
    |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        remove_if_exists(paths.settings_path()).map_err(PersistenceError::from)?;
        let committed = Arc::new(AppSettings::default());
        publish(snapshot, |state| {
            state.settings = Some(Cached::Loaded(committed.clone()))
        });
        Ok(committed)
    }
}

fn reset_subscriptions_job()
-> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<Arc<Vec<Subscription>>>
+ Send
+ 'static {
    |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        remove_if_exists(paths.subscriptions_path()).map_err(PersistenceError::from)?;
        let committed = Arc::new(Vec::new());
        publish(snapshot, |state| {
            state.subscriptions = Some(Cached::Loaded(committed.clone()))
        });
        Ok(committed)
    }
}

fn reset_manual_nodes_job()
-> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<Arc<Vec<ManualNode>>> + Send + 'static
{
    |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        remove_if_exists(paths.custom_nodes_path()).map_err(PersistenceError::from)?;
        let committed = Arc::new(Vec::new());
        publish(snapshot, |state| {
            state.manual_nodes = Some(Cached::Loaded(committed.clone()))
        });
        Ok(committed)
    }
}

/// Routing reset rewrites an empty rule set instead of unlinking the file; the
/// existing contract keeps the file present after the reset.
fn reset_routing_rules_job()
-> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<Arc<RoutingRuleSet>> + Send + 'static
{
    |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        let committed = Arc::new(RoutingRuleSet::new());
        persistence::save_routing_rules(paths, &committed)?;
        publish(snapshot, |state| {
            state.routing_rules = Some(Cached::Loaded(committed.clone()))
        });
        Ok(committed)
    }
}

/// Startup warm-up results, one entry per persisted store.
///
/// Every outcome is published into the shared snapshot before the receipt
/// resolves: a success becomes the committed value, and a failure is cached so
/// later readers never read the disk again and never substitute defaults for
/// corrupt data.
pub struct WarmResult {
    pub settings: Result<Arc<AppSettings>, PersistenceError>,
    pub subscriptions: Result<Arc<Vec<Subscription>>, PersistenceError>,
    pub manual_nodes: Result<Arc<Vec<ManualNode>>, PersistenceError>,
    pub routing_rules: Result<Arc<RoutingRuleSet>, PersistenceError>,
    pub latency: Result<Arc<LatencySnapshot>, PersistenceError>,
}

fn warm_job()
-> impl FnOnce(&AppPaths, &RwLock<Snapshot>) -> TransactionResult<WarmResult> + Send + 'static {
    |paths: &AppPaths, snapshot: &RwLock<Snapshot>| {
        Ok(WarmResult {
            settings: warm_store(
                snapshot,
                |state| state.settings.clone(),
                |state| &mut state.settings,
                || persistence::load_settings(paths),
            ),
            subscriptions: warm_store(
                snapshot,
                |state| state.subscriptions.clone(),
                |state| &mut state.subscriptions,
                || persistence::load_subscriptions(paths),
            ),
            manual_nodes: warm_store(
                snapshot,
                |state| state.manual_nodes.clone(),
                |state| &mut state.manual_nodes,
                || persistence::load_manual_nodes(paths),
            ),
            routing_rules: warm_store(
                snapshot,
                |state| state.routing_rules.clone(),
                |state| &mut state.routing_rules,
                || persistence::load_routing_rules(paths),
            ),
            latency: warm_store(
                snapshot,
                |state| state.latency.clone(),
                |state| &mut state.latency,
                || persistence::load_latency_snapshot(paths),
            ),
        })
    }
}

#[derive(Clone)]
pub struct WorkspaceStore {
    paths: AppPaths,
    snapshot: Arc<RwLock<Snapshot>>,
    writer: Arc<Writer>,
}

impl WorkspaceStore {
    pub fn new(paths: AppPaths) -> Self {
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let writer = Arc::new(Writer::new(paths.clone(), snapshot.clone()));
        Self {
            paths,
            snapshot,
            writer,
        }
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn load_settings(&self) -> Result<AppSettings, PersistenceError> {
        Ok((*self.settings_snapshot()?).clone())
    }

    pub fn settings_snapshot(&self) -> Result<Arc<AppSettings>, PersistenceError> {
        if let Some(cached) = with_snapshot(&self.snapshot, |state| state.settings.clone()) {
            return cached_result(cached);
        }
        let loaded = Arc::new(persistence::load_settings(&self.paths)?);
        publish_if_absent(&self.snapshot, loaded, |state| &mut state.settings)
    }

    pub fn load_subscriptions(&self) -> Result<Vec<Subscription>, PersistenceError> {
        Ok((*self.subscriptions_snapshot()?).clone())
    }

    pub fn subscriptions_snapshot(&self) -> Result<Arc<Vec<Subscription>>, PersistenceError> {
        if let Some(cached) = with_snapshot(&self.snapshot, |state| state.subscriptions.clone()) {
            return cached_result(cached);
        }
        let loaded = Arc::new(persistence::load_subscriptions(&self.paths)?);
        publish_if_absent(&self.snapshot, loaded, |state| &mut state.subscriptions)
    }

    pub fn load_manual_nodes(&self) -> Result<Vec<ManualNode>, PersistenceError> {
        Ok((*self.manual_nodes_snapshot()?).clone())
    }

    pub fn manual_nodes_snapshot(&self) -> Result<Arc<Vec<ManualNode>>, PersistenceError> {
        if let Some(cached) = with_snapshot(&self.snapshot, |state| state.manual_nodes.clone()) {
            return cached_result(cached);
        }
        let loaded = Arc::new(persistence::load_manual_nodes(&self.paths)?);
        publish_if_absent(&self.snapshot, loaded, |state| &mut state.manual_nodes)
    }

    pub fn load_manual_nodes_or_default(&self) -> Vec<ManualNode> {
        match self.load_manual_nodes() {
            Ok(nodes) => nodes,
            Err(err) => {
                log::warn!("load manual nodes: {err}");
                Vec::new()
            }
        }
    }

    pub fn load_routing_rules(&self) -> Result<RoutingRuleSet, PersistenceError> {
        Ok((*self.routing_rules_snapshot()?).clone())
    }

    pub fn routing_rules_snapshot(&self) -> Result<Arc<RoutingRuleSet>, PersistenceError> {
        if let Some(cached) = with_snapshot(&self.snapshot, |state| state.routing_rules.clone()) {
            return cached_result(cached);
        }
        let loaded = Arc::new(persistence::load_routing_rules(&self.paths)?);
        publish_if_absent(&self.snapshot, loaded, |state| &mut state.routing_rules)
    }

    pub fn load_latency_snapshot(&self) -> Result<LatencySnapshot, PersistenceError> {
        Ok((*self.latency_snapshot()?).clone())
    }

    pub fn latency_snapshot(&self) -> Result<Arc<LatencySnapshot>, PersistenceError> {
        if let Some(cached) = with_snapshot(&self.snapshot, |state| state.latency.clone()) {
            return cached_result(cached);
        }
        let loaded = Arc::new(persistence::load_latency_snapshot(&self.paths)?);
        publish_if_absent(&self.snapshot, loaded, |state| &mut state.latency)
    }

    /// Enqueues a whole save transaction and returns its receipt.
    pub(crate) fn enqueue_save_settings(
        &self,
        settings: AppSettings,
    ) -> PersistenceReceipt<Arc<AppSettings>> {
        self.writer.enqueue(save_settings_job(settings))
    }

    /// Mutates the latest committed settings inside the writer transaction.
    /// The closure must not call queue-submitting store methods: it already
    /// runs on the writer thread.
    pub(crate) fn enqueue_update_settings<F, R>(
        &self,
        mutate: F,
    ) -> PersistenceReceipt<(R, Arc<AppSettings>)>
    where
        F: FnOnce(&mut AppSettings) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.writer.enqueue(update_settings_job(mutate))
    }

    #[cfg(test)]
    pub(crate) fn enqueue_save_subscriptions(
        &self,
        subscriptions: Vec<Subscription>,
    ) -> PersistenceReceipt<Arc<Vec<Subscription>>> {
        self.writer.enqueue(save_subscriptions_job(subscriptions))
    }

    pub(crate) fn enqueue_update_subscriptions<F, R>(
        &self,
        mutate: F,
    ) -> PersistenceReceipt<(R, Arc<Vec<Subscription>>)>
    where
        F: FnOnce(&mut Vec<Subscription>) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.writer.enqueue(update_subscriptions_job(mutate))
    }

    #[cfg(test)]
    pub(crate) fn enqueue_save_manual_nodes(
        &self,
        nodes: Vec<ManualNode>,
    ) -> PersistenceReceipt<Arc<Vec<ManualNode>>> {
        self.writer.enqueue(save_manual_nodes_job(nodes))
    }

    pub(crate) fn enqueue_update_manual_nodes<F, R>(
        &self,
        mutate: F,
    ) -> PersistenceReceipt<(R, Arc<Vec<ManualNode>>)>
    where
        F: FnOnce(&mut Vec<ManualNode>) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.writer.enqueue(update_manual_nodes_job(mutate))
    }

    pub(crate) fn enqueue_save_routing_rules(
        &self,
        rules: RoutingRuleSet,
    ) -> PersistenceReceipt<Arc<RoutingRuleSet>> {
        self.writer.enqueue(save_routing_rules_job(rules))
    }

    #[cfg(test)]
    pub(crate) fn enqueue_save_latency_snapshot(
        &self,
        latency: LatencySnapshot,
    ) -> PersistenceReceipt<Arc<LatencySnapshot>> {
        self.writer.enqueue(save_latency_job(latency))
    }

    pub(crate) fn enqueue_update_latency_snapshot<F, R>(
        &self,
        mutate: F,
    ) -> PersistenceReceipt<(R, Arc<LatencySnapshot>)>
    where
        F: FnOnce(&mut LatencySnapshot) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.writer.enqueue(update_latency_job(mutate))
    }

    pub(crate) fn enqueue_reset_settings(&self) -> PersistenceReceipt<Arc<AppSettings>> {
        self.writer.enqueue(reset_settings_job())
    }

    pub(crate) fn enqueue_reset_subscriptions(&self) -> PersistenceReceipt<Arc<Vec<Subscription>>> {
        self.writer.enqueue(reset_subscriptions_job())
    }

    pub(crate) fn enqueue_reset_manual_nodes(&self) -> PersistenceReceipt<Arc<Vec<ManualNode>>> {
        self.writer.enqueue(reset_manual_nodes_job())
    }

    pub(crate) fn enqueue_reset_routing_rules(&self) -> PersistenceReceipt<Arc<RoutingRuleSet>> {
        self.writer.enqueue(reset_routing_rules_job())
    }

    /// Enqueues a no-op transaction that resolves after every earlier job has
    /// completed, including failed ones.
    pub(crate) fn enqueue_barrier(&self) -> PersistenceReceipt<()> {
        self.writer
            .enqueue(|_: &AppPaths, _: &RwLock<Snapshot>| Ok(()))
    }

    /// Warms every store on the writer thread. Call this after the instance
    /// lock, before the interactive app reads or writes the workspace.
    pub(crate) fn enqueue_warm(&self) -> PersistenceReceipt<WarmResult> {
        self.writer.enqueue(warm_job())
    }

    /// Synchronous startup warm for callers outside the GTK loop. It waits on
    /// the same receipt `enqueue_warm` returns, so the writer owns one reply
    /// path. Tokio blocking waits are avoided because the joined GTK tests warm
    /// the store with a runtime already entered.
    pub(crate) fn warm(&self) -> TransactionResult<WarmResult> {
        wait_for_receipt(self.enqueue_warm())
    }
}

#[cfg(test)]
impl WorkspaceStore {
    /// Test-only handle for holding an ordered write in flight. Shared with the
    /// GTK recovery tests; production builds do not compile the latch.
    pub(crate) fn test_writer_latch(&self) -> &Arc<WriterLatch> {
        &self.writer.latch
    }

    /// Test-only fixture seeds used by suites that build the workspace before
    /// launching the GTK app. They block on the same receipts the production
    /// enqueue path returns.
    pub(crate) fn save_subscriptions(
        &self,
        subscriptions: &[Subscription],
    ) -> Result<(), PersistenceError> {
        wait_for_receipt(self.enqueue_save_subscriptions(subscriptions.to_vec()))?;
        Ok(())
    }

    pub(crate) fn save_manual_nodes(&self, nodes: &[ManualNode]) -> Result<(), PersistenceError> {
        wait_for_receipt(self.enqueue_save_manual_nodes(nodes.to_vec()))?;
        Ok(())
    }

    pub(crate) fn save_latency_snapshot(
        &self,
        snapshot: &LatencySnapshot,
    ) -> Result<(), PersistenceError> {
        wait_for_receipt(self.enqueue_save_latency_snapshot(snapshot.clone()))?;
        Ok(())
    }
}

fn remove_if_exists(path: std::path::PathBuf) -> Result<(), std::io::Error> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use v2ray_rs_core::profile::AppProfile;

    fn store() -> (tempfile::TempDir, WorkspaceStore) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        paths.ensure_dirs().unwrap();
        (tmp, WorkspaceStore::new(paths))
    }

    fn commit<T>(receipt: PersistenceReceipt<T>) -> T {
        wait_for_receipt(receipt).expect("workspace writer stopped")
    }

    #[test]
    fn reads_share_one_snapshot_until_a_write_replaces_it() {
        let (_tmp, store) = store();
        let first = store.settings_snapshot().unwrap();
        let second = store.settings_snapshot().unwrap();
        assert!(Arc::ptr_eq(&first, &second));

        let mut settings = (*first).clone();
        settings.http_port = 2091;
        commit(store.enqueue_save_settings(settings));

        let third = store.settings_snapshot().unwrap();
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(third.http_port, 2091);
    }

    #[test]
    fn concurrent_updates_do_not_lose_writes() {
        let (_tmp, store) = store();
        let workers = 8;
        let per_worker = 25;
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for _ in 0..per_worker {
                        commit(store.enqueue_update_subscriptions(move |subscriptions| {
                            subscriptions.push(Subscription::new_from_url(
                                format!("worker-{worker}"),
                                "https://example.com/sub",
                            ));
                        }));
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(
            store.load_subscriptions().unwrap().len(),
            workers * per_worker
        );
        assert_eq!(
            persistence::load_subscriptions(store.paths())
                .unwrap()
                .len(),
            workers * per_worker
        );
    }

    #[test]
    fn concurrent_latency_updates_do_not_lose_writes() {
        let (_tmp, store) = store();
        let workers = 8;
        let per_worker = 25;
        let node_ref =
            |worker: usize, i: usize| v2ray_rs_core::models::ConnectionNodeRef::Subscription {
                subscription_id: Uuid::from_u128(worker as u128),
                node_id: Uuid::from_u128(i as u128),
            };
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for i in 0..per_worker {
                        commit(store.enqueue_update_latency_snapshot(move |snapshot| {
                            snapshot.upsert(node_ref(worker, i), 42, chrono::Utc::now());
                        }));
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let snapshot = store.load_latency_snapshot().unwrap();
        let persisted = persistence::load_latency_snapshot(store.paths()).unwrap();
        for worker in 0..workers {
            for i in 0..per_worker {
                let expected = node_ref(worker, i);
                assert!(
                    snapshot.get(expected).is_some(),
                    "in-memory snapshot missing worker {worker} sample {i}"
                );
                assert!(
                    persisted.get(expected).is_some(),
                    "latency_snapshot.json missing worker {worker} sample {i}"
                );
            }
        }
    }

    #[tokio::test]
    async fn queued_transactions_run_in_order_against_the_latest_commit() {
        let (_tmp, store) = store();
        store
            .enqueue_save_subscriptions(vec![Subscription::new_from_url(
                "A",
                "https://example.com/a",
            )])
            .await
            .unwrap()
            .unwrap();

        let first = store.enqueue_update_subscriptions(|subscriptions| {
            subscriptions.push(Subscription::new_from_url("B", "https://example.com/b"));
        });
        let second = store.enqueue_update_subscriptions(|subscriptions| {
            subscriptions.push(Subscription::new_from_url("C", "https://example.com/c"));
        });
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        let names: Vec<_> = store
            .load_subscriptions()
            .unwrap()
            .into_iter()
            .map(|subscription| subscription.name)
            .collect();
        assert_eq!(names, ["A", "B", "C"]);
        assert_eq!(
            persistence::load_subscriptions(store.paths())
                .unwrap()
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn a_failed_transaction_keeps_the_previous_snapshot() {
        let (_tmp, store) = store();
        let settings = AppSettings {
            http_port: 2091,
            ..Default::default()
        };
        store
            .enqueue_save_settings(settings)
            .await
            .unwrap()
            .unwrap();

        let receipt = store.enqueue_update_settings(|settings| {
            settings.antidpi.enabled = true;
            settings.antidpi.queue = 0;
        });
        let error = receipt.await.unwrap().unwrap_err();

        assert!(matches!(error, PersistenceError::InvalidSettings(_)));
        assert_eq!(store.settings_snapshot().unwrap().http_port, 2091);
    }

    #[tokio::test]
    async fn warm_reports_corrupt_settings_without_replacing_them() {
        let (_tmp, store) = store();
        std::fs::write(store.paths().settings_path(), b"not toml [").unwrap();
        store
            .enqueue_save_subscriptions(vec![Subscription::new_from_url(
                "Kept",
                "https://example.com/sub",
            )])
            .await
            .unwrap()
            .unwrap();

        let warmed = WorkspaceStore::new(store.paths().clone());
        let warm = warmed.enqueue_warm().await.unwrap().unwrap();

        assert!(warm.settings.is_err());
        assert_eq!(warm.subscriptions.unwrap().len(), 1);
        assert!(warmed.settings_snapshot().is_err());
        assert_eq!(
            std::fs::read(warmed.paths().settings_path()).unwrap(),
            b"not toml ["
        );
    }

    #[tokio::test]
    async fn a_warmed_failure_is_cached_instead_of_reread() {
        let (_tmp, store) = store();
        std::fs::write(store.paths().settings_path(), b"not toml [").unwrap();
        assert!(
            store
                .enqueue_warm()
                .await
                .unwrap()
                .unwrap()
                .settings
                .is_err()
        );

        // A reader after warm answers from the cached failure: the repaired
        // file is invisible to a warmed snapshot, so no disk read can turn a
        // corrupt store into defaults.
        persistence::save_settings(store.paths(), &AppSettings::default()).unwrap();
        assert!(store.settings_snapshot().is_err());

        // A write transaction must not silently adopt defaults either.
        let receipt = store.enqueue_update_settings(|settings| settings.http_port = 2091);
        assert!(matches!(
            receipt.await.unwrap(),
            Err(PersistenceError::CorruptConfig(_))
        ));
    }

    #[tokio::test]
    async fn a_closed_writer_answers_the_receipt_with_an_explicit_error() {
        let (tx, rx) = mpsc::channel::<Box<dyn Transaction>>();
        drop(rx);
        let writer = Writer {
            tx,
            latch: Arc::new(WriterLatch::default()),
        };
        let receipt = writer.enqueue(|_: &AppPaths, _: &RwLock<Snapshot>| Ok(()));
        drop(writer);

        match receipt.await.unwrap() {
            Err(PersistenceError::CorruptConfig(message)) => {
                assert_eq!(message, "workspace writer stopped");
            }
            other => panic!("expected writer error, got {other:?}"),
        }
    }

    #[test]
    fn reset_replaces_the_snapshot_and_removes_the_file() {
        let (_tmp, store) = store();
        commit(
            store.enqueue_save_subscriptions(vec![Subscription::new_from_url(
                "Example",
                "https://example.com/sub",
            )]),
        );
        assert_eq!(store.load_subscriptions().unwrap().len(), 1);

        commit(store.enqueue_reset_subscriptions());

        assert!(store.load_subscriptions().unwrap().is_empty());
        assert!(!store.paths().subscriptions_path().exists());
    }

    #[test]
    fn reset_routing_rules_recovers_from_corrupt_file() {
        let (_tmp, store) = store();
        std::fs::write(store.paths().routing_rules_path(), b"{corrupt json").unwrap();
        assert!(store.load_routing_rules().is_err());

        commit(store.enqueue_reset_routing_rules());

        let rules = store.load_routing_rules().unwrap();
        assert!(rules.rules().is_empty());

        let reloaded = WorkspaceStore::new(store.paths().clone())
            .load_routing_rules()
            .unwrap();
        assert!(reloaded.rules().is_empty());
    }

    #[test]
    fn reset_settings_recovers_from_a_corrupt_file() {
        let (_tmp, store) = store();
        let settings = AppSettings {
            http_port: 2091,
            ..Default::default()
        };
        commit(store.enqueue_save_settings(settings));
        std::fs::write(store.paths().settings_path(), b"not toml [").unwrap();
        assert!(persistence::load_settings(store.paths()).is_err());

        let reset = commit(store.enqueue_reset_settings());

        assert_eq!(*reset, AppSettings::default());
        assert_eq!(*store.settings_snapshot().unwrap(), AppSettings::default());
        assert!(!store.paths().settings_path().exists());

        let reloaded = WorkspaceStore::new(store.paths().clone())
            .load_settings()
            .unwrap();
        assert_eq!(reloaded, AppSettings::default());
    }

    #[test]
    fn reset_settings_treats_a_missing_file_like_a_present_one() {
        let (_tmp, store) = store();

        let reset = commit(store.enqueue_reset_settings());

        assert_eq!(*reset, AppSettings::default());
        assert!(!store.paths().settings_path().exists());
    }

    #[test]
    fn reset_settings_replaces_a_stale_cache_when_the_file_is_gone() {
        let (_tmp, store) = store();
        let settings = AppSettings {
            http_port: 2091,
            ..Default::default()
        };
        commit(store.enqueue_save_settings(settings));
        std::fs::remove_file(store.paths().settings_path()).unwrap();
        assert_eq!(store.settings_snapshot().unwrap().http_port, 2091);

        let reset = commit(store.enqueue_reset_settings());

        assert_eq!(*reset, AppSettings::default());
        assert_eq!(*store.settings_snapshot().unwrap(), AppSettings::default());
        assert!(!store.paths().settings_path().exists());
    }

    #[tokio::test]
    async fn a_held_transaction_keeps_the_previous_snapshot_and_queued_b_wins() {
        let (_tmp, store) = store();
        let base = AppSettings {
            http_port: 2090,
            ..Default::default()
        };
        store.enqueue_save_settings(base).await.unwrap().unwrap();
        let committed_before = store.settings_snapshot().unwrap();
        let disk_before = std::fs::read_to_string(store.paths().settings_path()).unwrap();

        store.test_writer_latch().arm();
        let a = store.enqueue_update_settings(|settings| settings.http_port = 2091);
        store.test_writer_latch().wait_until_parked();
        let b = store.enqueue_update_settings(|settings| settings.http_port = 2092);

        assert!(Arc::ptr_eq(
            &committed_before,
            &store.settings_snapshot().unwrap()
        ));
        assert_eq!(
            std::fs::read_to_string(store.paths().settings_path()).unwrap(),
            disk_before,
            "held A must not reach disk before its receipt"
        );

        store.test_writer_latch().release();
        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();

        assert_eq!(store.settings_snapshot().unwrap().http_port, 2092);
        assert_eq!(
            persistence::load_settings(store.paths()).unwrap().http_port,
            2092
        );
        assert_eq!(
            WorkspaceStore::new(store.paths().clone())
                .load_settings()
                .unwrap()
                .http_port,
            2092,
            "B must survive a reopen"
        );
    }

    #[tokio::test]
    async fn a_failing_transaction_does_not_block_queued_b_and_b_survives_reopen() {
        let (_tmp, store) = store();
        let base = AppSettings {
            http_port: 2090,
            ..Default::default()
        };
        store.enqueue_save_settings(base).await.unwrap().unwrap();
        let committed_before = store.settings_snapshot().unwrap();
        let default_antidpi = AppSettings::default().antidpi;

        store.test_writer_latch().arm();
        let a = store.enqueue_update_settings(|settings| {
            settings.antidpi.enabled = true;
            settings.antidpi.queue = 0;
        });
        store.test_writer_latch().wait_until_parked();
        let b = store.enqueue_update_settings(|settings| settings.http_port = 2093);
        assert!(Arc::ptr_eq(
            &committed_before,
            &store.settings_snapshot().unwrap()
        ));

        store.test_writer_latch().release();
        assert!(matches!(
            a.await.unwrap(),
            Err(PersistenceError::InvalidSettings(_))
        ));
        b.await.unwrap().unwrap();

        let committed = store.settings_snapshot().unwrap();
        assert_eq!(committed.http_port, 2093);
        assert_eq!(
            committed.antidpi, default_antidpi,
            "failed A must not reach the committed snapshot"
        );
        let on_disk = persistence::load_settings(store.paths()).unwrap();
        assert_eq!(on_disk.http_port, 2093);
        assert_eq!(on_disk.antidpi, default_antidpi);
        assert_eq!(
            WorkspaceStore::new(store.paths().clone())
                .load_settings()
                .unwrap()
                .http_port,
            2093
        );
    }

    #[tokio::test]
    async fn a_queued_save_never_recreates_a_subscriptions_reset_file() {
        let (_tmp, store) = store();
        store
            .enqueue_save_subscriptions(vec![Subscription::new_from_url(
                "Seed",
                "https://example.com/seed",
            )])
            .await
            .unwrap()
            .unwrap();

        store.test_writer_latch().arm();
        let save = store.enqueue_save_subscriptions(vec![
            Subscription::new_from_url("One", "https://example.com/one"),
            Subscription::new_from_url("Two", "https://example.com/two"),
        ]);
        store.test_writer_latch().wait_until_parked();
        let reset = store.enqueue_reset_subscriptions();
        assert!(store.paths().subscriptions_path().exists());

        store.test_writer_latch().release();
        save.await.unwrap().unwrap();
        reset.await.unwrap().unwrap();

        assert!(store.load_subscriptions().unwrap().is_empty());
        assert!(!store.paths().subscriptions_path().exists());
        assert!(
            WorkspaceStore::new(store.paths().clone())
                .load_subscriptions()
                .unwrap()
                .is_empty(),
            "reset must survive a reopen"
        );
    }
}
