# ⚡️ Valohai Conduit

## Usage

- TODO(?): publish on https://crates.io/
- TODO(?): build and publish on GitHub
- TODO(?): add script/guidance how to install straight from GitHub releases

```shell
cargo install valohai-conduit
conduit --help

# TODO: document the configuration

conduit
# then you can already make requests through the proxy, try:
# => http://127.0.0.1:8080/openai
# => http://127.0.0.1:8080/anthropic/v1/messages
```

## Development

### Prerequisites

- Rust version specified at `rust-toolchain.toml` / `mise.toml`
- latest [prek](https://github.com/j178/prek) for pre-commit hooks
- _(optional)_ [uv](https://github.com/uv/uv) for running Python examples to seed data

```shell
mise install          # installs Rust, prek and uv
# or install Rust (rustup) manually: https://rust-lang.org/install
# or install prek manually: https://prek.j178.dev/installation/
# or install uv manually: https://docs.astral.sh/uv/getting-started/installation/
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

### Python Examples

```shell
# configure and run Conduit, e.g.:
# cp conduit.example.toml conduit.toml
# vim conduit.toml
# cargo run -- -vv

# while it's running, in a separate terminal:
cd examples/
cp .env.example .env
vim .env
uv run --env-file .env python/anthropic_messages.py
uv run --env-file .env python/anthropic_messages_streaming.py
uv run --env-file .env python/openai_chat_completions.py
uv run --env-file .env python/openai_chat_completions_streaming.py
uv run --env-file .env python/openai_completions.py
uv run --env-file .env python/openai_completions_streaming.py
uv run --env-file .env python/openai_responses.py
uv run --env-file .env python/openai_responses_streaming.py
```
