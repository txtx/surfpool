# Transaction Ingestion Benchmarks

Benchmarks for `send_transaction` performance across different transaction types.

## Run

```bash
cargo bench --bench transaction_ingestion -p surfpool-core
```

## Benchmarks

- `simple_transfer` - 1 instruction
- `multi_instruction_transfer` - 5 instructions  
- `large_transfer` - 10 instructions
- `complex_with_compute_budget` - compute budget + transfers
- `kamino_strategy` - protocol-like simulation (16+ instructions, high CU)

Component overhead:
- `transaction_deserialization` - decode overhead
- `transaction_serialization` - encode overhead
- `clone_overhead_string` - String clone cost
- `clone_overhead_context` - Context clone cost

## Protocol Fixtures

To benchmark real protocol transactions, add fixture files:

```
crates/core/benches/fixtures/protocols/{protocol}_transactions.json
```

Format: JSON array of base58-encoded transaction strings.

The benchmark automatically detects and runs protocol benchmarks when fixtures exist.
