## 1. Scheduling and dispatch

- [x] 1.1 Add a 10-minute session-local timer in `SubscriptionsPage`
- [x] 1.2 Route timer ticks through the same async command path used by manual latency tests
- [x] 1.3 Skip subscriptions already under test and ignore disabled subscriptions or nodes

## 2. Shared result handling

- [x] 2.1 Reuse `SubscriptionsCmdOutput::LatencyResult` for manual and scheduled refreshes
- [x] 2.2 Persist updated samples to `latency_snapshot.json`
- [x] 2.3 Keep `last_latency_ms` in sync with the latest completed sample for UI rendering

## 3. Verification

- [x] 3.1 Verify latency refresh works while the backend is connected without disconnecting or restarting it
- [x] 3.2 Verify scheduled refresh updates the stored latency inputs used by later lowest-latency connection attempts

## 4. Startup hydration

- [x] 4.1 On init, load `LatencySnapshot` and populate `last_latency_ms` for matching subscription nodes so the UI shows latency immediately on startup
