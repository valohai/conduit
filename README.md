# ⚡️ Valohai Conduit

## Development

Install Rust version specified at `rust-toolchain.toml` / `mise.toml`
in the way you prefer.

Install [prek](https://github.com/j178/prek) for pre-commit hooks:

```shell
mise install          # installs both Rust and prek
# or install prek manually: https://prek.j178.dev/installation/
```

Then set up the hooks:

```shell
prek install
```

To run the pre-commit hooks manually on all files:

```shell
prek --all-files
# or, linting separately:
# cargo fmt
# cargo clippy --workspace --tests
```

You're good to go:

```shell
cargo run -- help
```

## Usage

- TODO(?): publish on https://crates.io/
- TODO(?): build and publish on GitHub
- TODO(?): add script/guidance how to install straight from GitHub releases

```shell
cargo install valohai-conduit
conduit --help
```
