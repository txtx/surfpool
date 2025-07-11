## How to use

After commpiling the `hello-geyser` plugin:
```console
cd examples/hello-geyser
cargo build --release
cd ../..
```

The plugin can be attached using the command:
```console
surfpool start -g examples/hello-geyser/hello-geyser.json
```
