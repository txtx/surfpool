# Transaction Ingestion Benchmarks

Performance benchmarks for `send_transaction` across different transaction types.

## Usage

```bash
cargo bench --bench transaction_ingestion -p surfpool-core
```

## Benchmarks

### Transaction Types
- `simple_transfer` - Single transfer instruction
- `multi_instruction_transfer` - 5 transfer instructions
- `large_transfer` - 10 transfer instructions
- `complex_with_compute_budget` - Compute budget + transfers
- `kamino_strategy` - Protocol-like with multi-sig (12+ instructions)

### Component Overhead
- `transaction_deserialization` - Decode from base58/bincode
- `transaction_serialization` - Encode to base58/bincode
- `clone_overhead_string` - Transaction string clone cost
- `clone_overhead_context` - RunloopContext clone cost

## Implementation

Each benchmark pre-generates a pool of 100 transactions. Transactions are rotated during measurement to isolate ingestion performance from generation overhead. After exhausting unique transactions (~100-200 iterations), duplicate submissions are handled gracefully, which does not affect statistical validity for regression tracking.

## Protocol Fixtures

Optional: Add real protocol transaction fixtures for additional benchmarks.

```
crates/core/benches/fixtures/protocols/{protocol}_transactions.json
```

Format: JSON array of base58-encoded serialized transactions.
