# ⚡️ Valohai Conduit

## Usage

- TODO(?): publish on https://crates.io/
- TODO(?): build and publish on GitHub
- TODO(?): add script/guidance how to install straight from GitHub releases

```shell
cargo install valohai-conduit
conduit --help
```

## Development

### Prerequisites

- Rust version specified at `rust-toolchain.toml` / `mise.toml`
- latest [prek](https://github.com/j178/prek) for pre-commit hooks

```shell
mise install          # installs both Rust and prek
# or install Rust (rustup) manually: https://rust-lang.org/install
# or install prek manually: https://prek.j178.dev/installation/
```

### Setup

```shell
prek install          # set up pre-commit hooks
cargo run -- help     # verify it works
```

### Linting

```shell
prek --all-files
# or separately:
# cargo fmt
# cargo clippy --workspace --tests
```

### Testing

```shell
cargo test --workspace
```
