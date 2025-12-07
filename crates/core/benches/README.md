# Transaction Ingestion Benchmarks

Performance benchmarks for measuring Surfpool's transaction processing capabilities.

## Quick Start

```bash
cargo bench --bench transaction_ingestion -p surfpool-core
```

## What We Measure

### Transaction Ingestion
- `simple_transfer` - Single SOL transfer instruction
- `multi_instruction_transfer` - 5 SOL transfer instructions
- `large_transfer` - 10 SOL transfer instructions

### Component Overhead
- `transaction_deserialization` - Base58 decode + bincode deserialize
- `transaction_serialization` - Transaction creation + Base58 encode
- `clone_overhead_string` - String cloning cost (API constraint)
- `clone_overhead_context` - Context cloning cost (API constraint)

## Understanding Results

The `send_transaction` benchmarks include unavoidable API overhead from cloning `RunloopContext` and transaction strings. To calculate pure transaction processing time, subtract the clone overhead from the measured time.

## Current Limitations

This initial implementation focuses on **simple SOL transfers** to establish baseline performance. Complex DeFi transactions (Kamino, Jupiter, etc.) will be added in future phases.
