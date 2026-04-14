# `surfpool-sdk`

Node.js bindings for the Surfpool SDK, built with `napi-rs`.

## Development

```bash
npm ci
npm run build
npm run smoke
```

## Publishing

The npm package is released from `crates/sdk-node` using prebuilt native artifacts for:

- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`

The GitHub Actions release workflow builds those artifacts first, then runs `napi prepublish -t npm` before `npm publish`.
