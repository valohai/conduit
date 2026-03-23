# Contributing

## Development

### Prerequisites

- _(optional)_ [mise](https://mise.jdx.dev/installing-mise.html) to manage all of the below
- Rust version specified at `rust-toolchain.toml` / `mise.toml`
- latest [prek](https://github.com/j178/prek) for pre-commit hooks
- _(optional)_ [uv](https://github.com/uv/uv) for running Python examples to seed data

```shell
mise install          # installs Rust, prek and uv
# or install Rust (rustup) manually: https://rust-lang.org/install
# or install prek manually: https://prek.j178.dev/installation/
# or install uv manually: https://docs.astral.sh/uv/getting-started/installation/
```

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

### Running

To run the proxy:

```shell
cp conduit.example.toml conduit.toml
vim conduit.toml
cargo run -- proxy
```

Then, while the proxy is running, you can run the examples documented further
below to get tracking data.

To run the terminal-based dashboard, in a separate terminal:

```shell
cargo run -- dashboard
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
