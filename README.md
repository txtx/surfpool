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

### Getting Started

Install `surfpool` from homebrew:

```bash
brew install txtx/taps/surfpool
```

Alternatively, you can clone the repo and build from source.

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

`surfpool` is in active development, if you'd like to get involved, you can:

- Take a stab at any of these issues: https://github.com/txtx/surfpool/issues?q=is%3Aissue%20state%3Aopen%20label%3A%22help%20wanted%22
- Join the telegram channel https://t.me/surfpool

