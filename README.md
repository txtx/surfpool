<div align="center">
  <picture>
    <source srcset="https://raw.githubusercontent.com/txtx/surfpool/main/docs/assets/surfpool-hero.png">
    <img alt="surfpool" style="max-width: 60%;">
  </picture>
</div>

# surfpool

### TL;DR

`surfpool` is to solana what `anvil` is to ethereum: a blazing fast in-memory testnet that has the ability to point-fork Solana mainnet instantly.

### Design

`surfpool` uses the low level `solana-svm` API through the excellent wrapper [LiteSVM](https://github.com/LiteSVM/litesvm).
This approach provides greater flexibility and significantly faster boot times.

`surfpool` is also available as a library on crates.io - facilitating its integration in other projects.

### Getting Started

Install `surfpool` from crates.io:

```console
$ cargo install surfpool-cli
```

Start a local validator / simnet:

```console
$ surfpool run
```

<div align="center">
  <picture>
    <source srcset="https://raw.githubusercontent.com/txtx/surfpool/main/docs/assets/screenshot.png">
    <img alt="surfpool" style="max-width: 60%;">
  </picture>
</div>

## Status

`surfpool` is in active development, here are a few todos of what's coming next.

### RPC Support

- [ ] Support `HTTP Methods`
    - Priority 1:
        - [x] getVersion
        - [x] getSlot
        - [x] getEpochInfo
        - [x] getLatestBlockhash
        - [x] sendTransaction
        - [x] getAccountInfo
        - [x] getMultipleAccounts
        - [x] getFeeForMessage
        - [ ] getSignatureStatuses
        - [ ] getMinimumBalanceForRentExemption
        - [ ] requestAirdrop
        - [ ] getBalance
        - [ ] simulateTransaction
        - [ ] getTransaction
    - Priority 2:
        - [ ] getBlock
        - [ ] getBlockCommitment
        - [ ] getBlockHeight
        - [ ] getBlockProduction
        - [ ] getBlocks
        - [ ] getBlocksWithLimit
        - [ ] getBlockTime
        - [ ] getClusterNodes
        - [ ] getEpochSchedule
        - [ ] getFirstAvailableBlock
        - [ ] getGenesisHash
        - [x] getHealth
        - [ ] getHighestSnapshotSlot
        - [ ] getIdentity
        - [ ] getInflationGovernor
        - [ ] getInflationRate
        - [ ] getInflationReward
        - [ ] getLargestAccounts
        - [ ] getLeaderSchedule
        - [ ] getMaxRetransmitSlot
        - [ ] getMaxShredInsertSlot
        - [ ] getMinimumBalanceForRentExemption
        - [ ] getProgramAccounts
        - [ ] getRecentPerformanceSamples
        - [ ] getRecentPrioritizationFees
        - [ ] getSignaturesForAddress
        - [ ] getSlotLeader
        - [ ] getSlotLeaders
        - [ ] getStakeMinimumDelegation
        - [ ] getSupply
        - [ ] getTokenAccountBalance
        - [ ] getTokenAccountsByDelegate
        - [ ] getTokenAccountsByOwner
        - [ ] getTokenLargestAccounts
        - [ ] getTokenSupply
        - [ ] getTransactionCount
        - [ ] getVoteAccounts
        - [ ] isBlockhashValid
        - [ ] minimumLedgerSlot
- [ ] Support `Websocket Methods`

### Configurations

- [ ] Ability to pull configuration from a `Surfpool.toml` manifest file
- [ ] Ability to setup slot time (default to `400ms`)
- [ ] Ability to setup epoch duration (default to `432,000`)
- [ ] Ability to configure behavior:
    - [ ] genesis
    - [ ] point-fork mainnet
    - [ ] stream-fork mainnet
- [ ] Ability to configure the RPC node URL to default to.

### Going Further
- [ ] Ability to watch and update fetched account
