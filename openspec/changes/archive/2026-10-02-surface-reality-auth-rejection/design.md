## Context

- `crates/ui/src/backend_warning.rs` already matches backend lines per backend against a pattern table (`PatternId` incl. the REALITY `potential MITM or redirection` line) and toasts the first match once per connection — the detection half of this change exists; only the failure-path use and the classification text are new.
- `crates/ui/src/connection.rs` drives candidates in order and declares a candidate failed once its manager lands in the `Error` state; the manager's `LogBuffer` then holds the candidate's whole captured output, because the manager's exit path drains its log readers before the state transition. Classification reads that buffer at the moment the candidate is marked failed. (The log forwarder that feeds the warning toast keeps its own copy of the lines, but that copy can lag the failure report, so it is not the classification source.)
- The measured failure shape: TCP to the server connects, the REALITY handshake is rejected, the backend repeats the signature, and the candidate times out — the signature is always in the buffer by the time the failure is reported.

## Decisions

- **Classify at candidate-failure time, not per log line.** The existing toast fires per line for warnings; this change classifies once when the candidate fails, matching against the candidate's whole captured output. A warning toast may already have appeared for the same line; the classified failure report is the actionable one and replaces the generic failure text rather than stacking a second toast.
- **One classification now, table-extensible later.** The signature set starts with REALITY auth rejection only; the match function takes `(backend, captured output) -> Option<ClassifiedFailure>` so future signatures (DNS bootstrap timeout, TLS verification refusal) slot in without touching the reporting path.
- **The refresh offer rides the classified report as an action**, not a notification of its own: one toast with "Refresh subscription" opening the existing manual update flow for that subscription. At most once per connection per subscription covers failover across several nodes of the same subscription.
- **Never auto-refresh.** Key rotation is a provider-side event that a refresh may or may not fix (measured live: a same-day refresh still served rejected keys); auto-refreshing on every rejection would hammer the provider during an outage. The offer is the fix, the user pulls the trigger.

## Risks

- A server that misconfigures REALITY for everyone (not a key rotation) shows the same signature; the cause text says "keys may be stale", which stays true either way — refresh is the user's cheapest first move and reporting the provider is the second.
- Providers rotating keys mid-connection would re-trigger classification on every connect; the per-connection and per-subscription dedup caps keep this to one report and one offer per incident.
