## Why

A connection fails over to the next candidate whenever a candidate exhausts its crash budget, even when the failure has nothing to do with the node. On 2026-09-14 a sing-box TUN startup FATAL caused by the host kernel (`add rule 0/9: address family not supported by protocol`) was retried 3 times on each of 7 nodes, about 40 seconds in total. The final error listed node names, so it looked like seven bad nodes rather than one host problem. Backend startup failures caused by the host, the binary, or the generated config repeat identically on every candidate.

## What Changes

- When two consecutive candidates of one connection attempt fail with the same reason (once node-specific parts and volatile prefixes are removed), the connection stops failing over and ends in `Error`.
- That `Error` states the shared reason once and says it repeated on consecutive nodes and is not node-specific, instead of the per-node list.
- Failure text shown to the user drops ANSI color escapes (sing-box prints `\x1b[31mFATAL\x1b[0m`).
- Failover for differing, node-specific failures is unchanged, and so is a direct connect (single candidate).

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `process-lifecycle`: "Connection terminal state has a single source" gains the early-stop rule for repeated identical failures and its summary text.

## Impact

- `crates/ui/src/connection.rs` — failure-reason normalization, early stop in the candidate loop, `summarize_failures` text.
- No change to `ProcessManager` crash budgeting or to the candidate planner.
