# msfs-utils

Typed asynchronous and blocking Rust clients for Microsoft Flight Simulator's
SimConnect API, plus platform-independent replay utilities.

The workspace contains:

- `msfs-async`: a runtime-independent async client backed by a dedicated
  Windows event thread;
- `msfs-sync`: a blocking facade over the same driver;
- `msfs-async-derive`: validated simulation-object and client-data definition
  macros;
- `msfs-replay`: CSV recording validation and aircraft-pose interpolation.

The SimConnect clients require Windows and the MSFS SDK. Macro and replay tests
run on other platforms.

The workspace currently uses a pinned checkout of `msfs-rs`:

```console
git clone https://github.com/flybywiresim/msfs-rs checkouts/msfs-rs
git -C checkouts/msfs-rs checkout 2f697b9aac9fa3c00474f901a7f7ee4218cf534b
```

## Development

```console
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

See the README in each crate for API examples and platform-specific setup.
