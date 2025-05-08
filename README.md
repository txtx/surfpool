<div align="center">
  <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/txtx/surfpool/main/doc/assets/surfpool-github-hero-dark.png">
      <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/txtx/surfpool/main/doc/assets/surfpool-github-hero-light.png">
      <img alt="Surfpool is the best place to train before surfing Solana" style="max-width: 60%;">
  </picture>
</div>

### TL;DR

`surfpool` is to Solana what `anvil` is to Ethereum: a blazing fast ⚡️ in-memory testnet that has the ability to point-fork Solana mainnet instantly.

## Introduction

Surfpool provides a blazing-fast, developer-friendly simulation of Solana Mainnet that runs seamlessly on your local machine. It eliminates the need for high-performance hardware while maintaining an authentic testing environment.

Whether you're developing, debugging, or educating yourself on Solana, Surfpool gives you an instant, self-contained network that dynamically fetches missing Mainnet data as needed—no more manual account setups.

## Surfpool in action: 101 Series 

<a href="https://www.youtube.com/playlist?list=PL0FMgRjJMRzO1FdunpMS-aUS4GNkgyr3T">
  <picture>
    <source srcset="https://raw.githubusercontent.com/txtx/surfpool/main/doc/assets/youtube.png">
    <img alt="Surfpool 101 series" style="max-width: 100%;">
  </picture>
</a>

## Features

- Fast & Lightweight – Runs smoothly on any machine without heavy system requirements.

- Dynamic Account Fetching – Automatically retrieves necessary Mainnet accounts during transaction execution.

- Anchor Integration – Detects Anchor projects and deploys programs automatically.

- Educational & Debug-Friendly – Provides clear insights into transaction execution and state changes.

- Easy Installation – Available via Homebrew, Snap, and direct binaries.

## Installation

Install Surfpool CLI with:

```console
# macOS (Homebrew)
brew install txtx/taps/surfpool

# Linux (Snapstore)
snap install surfpool
```

Verify installation:

```console
surfpool --version
```

## Usage

Start a local Solana network with:

```console
surfpool start
```

If inside an Anchor project, Surfpool will:

- Automatically generate infrastructure as code (similar to Terraform).

- Deploy your Solana programs to the local network.

- Provide a clean, structured environment to iterate safely.

The command:

```console
surfpool start --help
```

Is documenting all the options available.

## Crypto Infrastructure as Code: A New Standard in Web3

Infrastructure as code (IaC) transforms how teams deploy and operate Solana programs:

- Declarative & Reproducible – Clearly defines environments, making deployments consistent.

- Auditable – Security teams can review not just the code of your Solana programs, but the way you will be deploying and operating your protocol.

- Seamless Transition to Mainnet – Test with the exact infrastructure that will go live.

With Surfpool, every developer learns to deploy Solana programs the right way—scalable, secure, and production-ready from day one.


## Architecture & How to Contribute

Surfpool is built on the low-level solana-svm API, utilizing the excellent LiteSVM wrapper. This approach provides greater flexibility and significantly faster boot times, ensuring a smooth developer experience.

We are actively developing Surfpool and welcome contributions from the community. If you'd like to get involved, here’s how:

- Explore and contribute to open issues: [GitHub Issues](https://github.com/txtx/surfpool/issues?q=is%3Aissue%20state%3Aopen%20label%3A%22help%20wanted%22)

- Join the discussion on [Discord](https://discord.gg/VxZhMqdd)

- Get releases updates via [X](https://x.com/txtx_sol) or [Telegram Channel](https://t.me/surfpool)

Your contributions help shape the future of Surfpool, making it an essential tool for Solana developers worldwide.
