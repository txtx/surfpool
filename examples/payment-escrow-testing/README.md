# Payment Escrow Testing with Surfpool

Test time-locked escrows without waiting hours/days for locks to expire.

## What This Covers

- Time-locked releases
- Grace periods
- Multi-party settlement
- Block time edge cases

Works for vesting, escrows, auctions, staking - anything with time constraints.

## Running the Examples

```bash
# Start Surfpool
surfpool

# Run examples
npx ts-node escrow-lifecycle.ts
npx ts-node time-lock-scenarios.ts
```

## Examples

### escrow-lifecycle.ts

Full lifecycle: create -> attempt early release (fails) -> advance time -> release -> dispute -> expiration.

### time-lock-scenarios.ts

Edge cases: exact expiry boundaries, staggered locks, grace periods, snapshot/restore for testing alternate paths.

## Key Patterns

### Time Travel for Lock Testing

```typescript
// Create escrow with 24h lock
const escrow = await createEscrow(provider, 86400);

// Try release before lock (should fail)
await advanceTime(3600); // 1 hour
const earlyRelease = await tryRelease(escrow);
assert(!earlyRelease.success);

// Advance past lock
await advanceTime(86400); // +24 hours
const lateRelease = await tryRelease(escrow);
assert(lateRelease.success);
```

### Snapshot/Restore for Branch Testing

```typescript
// Snapshot before critical decision
const snapshot = await surfpool.snapshot();

// Test dispute path
await dispute(escrow);
const disputeOutcome = await resolve(escrow);

// Restore and test release path
await surfpool.restore(snapshot);
await release(escrow);
```

### Parallel Scenario Testing

```typescript
// Create multiple escrows with different locks
const escrows = await Promise.all([
  createEscrow(provider, 3600),   // 1 hour
  createEscrow(provider, 86400),  // 24 hours
  createEscrow(provider, 604800), // 7 days
]);

// Advance 2 hours - only first should be releasable
await advanceTime(7200);

for (const escrow of escrows) {
  const canRelease = await checkReleasable(escrow);
  console.log(`${escrow.lockSeconds}s lock: ${canRelease}`);
}
```

## Anchor Integration

Swap the mock escrow interface for your program's accounts:

```typescript
interface Escrow {
  pda: PublicKey;
  agent: PublicKey;
  provider: PublicKey;
  amount: BN;
  createdAt: BN;
  expiresAt: BN;
  status: 'active' | 'released' | 'disputed' | 'expired';
}
```

## Notes

- Snapshots are cheap, use them to test multiple paths from same state
- Test T-1, T, T+1 around expiry boundaries
- Advance a few slots between ops to simulate network delay
- Fork mainnet to test against real balances
