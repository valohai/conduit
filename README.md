# Valohai Conduit

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A local proxy that sits between your code and LLM providers,
recording billable usage. Point SDK's base URL at Conduit, and get detailed
tracking with zero code changes.

Single binary, written in Rust.

## Usage

- TODO(?): publish on https://crates.io/
- TODO(?): build and publish on GitHub
- TODO(?): add script/guidance how to install straight from GitHub releases

```shell
cargo install valohai-conduit
conduit --help

# TODO: document the configuration

conduit
```

**Once Conduit is running, point your SDK's base URL at it.**
The URL is `http://<proxy-address>/<provider_key>` where `<provider_key>` matches
a `[providers.<key>]` section in your `conduit.toml`.

Set the base URL environment variable for your provider SDK:

```shell
# NB: OpenAI-like providers also need the `/v1` path suffix:
export OPENAI_BASE_URL=http://localhost:8080/openai/v1
export ANTHROPIC_BASE_URL=http://localhost:8080/anthropic
```

Most SDKs pick these up automatically — no code changes needed.

Alternatively, pass `base_url` directly:

```python
client = OpenAI(base_url="http://localhost:8080/openai/v1")
client = Anthropic(base_url="http://localhost:8080/anthropic")
```

See [`examples/`](examples/) for complete working examples.

## Development

See the
[contributing guide](https://github.com/valohai/conduit?tab=contributing-ov-file)
to get started.
