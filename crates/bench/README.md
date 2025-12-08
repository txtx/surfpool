# Transaction Ingestion Benchmarks

Performance benchmarks for `send_transaction` across different transaction types and component overhead.

## Usage

```bash
cargo bench --bench transaction_ingestion -p surfpool-bench
```

## Benchmarks

### Transaction Ingestion (RPC send_transaction)
- `simple_transfer` - Single transfer instruction
- `multi_instruction_transfer` - 5 transfer instructions
- `large_transfer` - 10 transfer instructions (2x airdrop)
- `complex_with_compute_budget` - Transaction with compute budget instruction + 5 transfers
- `kamino_strategy` - Protocol-like strategy with intermediate keypairs (12+ instructions)

These benchmarks measure the end-to-end performance of the `send_transaction` RPC call with different transaction complexities, from simple transfers to complex multi-step strategies.

### Transaction Component Overhead
- `transaction_deserialization` - Decode from base58/bincode
- `transaction_serialization` - Encode to base58/bincode
- `clone_overhead_string` - Transaction string clone cost
- `clone_overhead_context` - RunloopContext clone cost

These benchmarks isolate specific transaction processing operations to measure their individual performance characteristics.

## Implementation

**Transaction Ingestion:**
- Pre-generates a pool of transactions to avoid generation overhead during measurement
- Uses a single shared runloop to avoid thread accumulation
- Measures end-to-end `send_transaction` performance from submission to result
- Each benchmark runs with 10 samples, 1 second warmup, and 1 second measurement time

**Components:**
- Directly measures serialization/deserialization and clone operations
- No runloop dependency for accurate baseline measurements
- Same sample/warmup/measurement configuration for consistency

## Known Limitations

The `send_transaction` benchmarks depend on proper runloop operation. If the runloop gets stuck or unresponsive, benchmarks may take longer but will eventually complete or timeout.
