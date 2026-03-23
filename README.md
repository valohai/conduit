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
# then you can already make requests through the proxy, try:
# => http://127.0.0.1:8080/openai
# => http://127.0.0.1:8080/anthropic/v1/messages
```

## Development

See the
[contributing guide](https://github.com/valohai/conduit?tab=contributing-ov-file)
to get started.
