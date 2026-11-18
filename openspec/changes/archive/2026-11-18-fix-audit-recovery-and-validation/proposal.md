# Proposal

## Why
Recovery can report success with routing state still installed. Subscription failures can expose server content and imports can block workers.

## What Changes
- Propagate cleanup failures and retain failed recovery journals.
- **BREAKING**: Require journal and session identity for automatic routing; legacy broad cleanup requires an explicit operator override.
- Match subscription connection identities before endpoint fallbacks.
- Bound local imports and exclude response bodies from routine errors.
- Preserve generated configuration on routing load failure.
- Strengthen delivery checks and correct operational documentation.

## Capabilities
### New Capabilities
None.
### Modified Capabilities
- `tun-mode`: Failed recovery remains pending; automatic helper operations require durable ownership.
- `app-persistence`: Versioned TUN journals preserve ownership and unknown outcomes.
- `subscription-update`: Stable matching and bounded imports.

## Impact
Route helper, desktop recovery, subscription updates, CI, release workflow, and documentation. Reuse existing workspace dependencies.
