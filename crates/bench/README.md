# Transaction Processing Benchmarks

Performance benchmarks for transaction processing components.

## Usage

```bash
cargo bench --bench transaction_ingestion -p surfpool-bench
```

## Benchmarks

### Transaction Component Overhead
- `transaction_deserialization` - Decode from base58/bincode
- `transaction_serialization` - Encode to base58/bincode
- `clone_overhead_string` - Transaction string clone cost
- `clone_overhead_context` - RunloopContext clone cost

These benchmarks measure the overhead of fundamental transaction operations without requiring the runloop infrastructure.

## Implementation

The component benchmarks isolate specific transaction processing operations to measure their individual performance characteristics. Each benchmark runs with 10 samples, 1 second warmup, and 1 second measurement time.

## Future: Full Transaction Ingestion

Transaction ingestion benchmarks through the full RPC pipeline require a working runloop infrastructure. The current architecture of `start_local_surfnet_runloop()` has limitations that prevent reliable benchmarking. Once the runloop provides explicit shutdown support, comprehensive transaction ingestion benchmarks can be implemented.
