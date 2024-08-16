## Check the MUSL tools installed
```bash
sudo apt-get install musl-tools

```

## Build your project:
Use the cargo build command with the --target flag to specify the MUSL target:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

### This will create a statically linked binary in the target/x86_64-unknown-linux-musl/release/ directory.





