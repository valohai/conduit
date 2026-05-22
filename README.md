# Valohai Conduit

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Conduit is a local proxy that sits between your code and LLM providers
recording billable usage.\
Point SDK's base URL at Conduit, and get detailed
tracking with zero code changes.

Single binary, written in Rust.

What Conduit allows you to do

- run a proxy locally to intercept API calls
- set up one or more LLM API providers to forward the API calls to
- view the token consumption of each API call in a terminal UI
- send information to Valohai LLM for further cost analysis

## Install

Conduit is available through:

- installer script or prebuilt binary from [GitHub Releases](https://github.com/valohai/conduit/releases/latest)
- `cargo install valohai-conduit` from [crates.io](https://crates.io/crates/valohai-conduit) (requires Rust)

You can confirm that Conduit is installed with:

```shell
conduit --help

# Usage: conduit [OPTIONS] [COMMAND]
#
# Commands:
#   proxy      Run the proxy server [default command]
#   dashboard  Open the terminal-based dashboard
#   help       Print this message or the help of the given subcommand(s)
```

## Configuration

Conduit uses a `conduit.toml` file as its configuration file.

Create a `conduit.toml` file in your working directory, otherwise Conduit will not be able to run.

You can copy the example [from here](https://github.com/valohai/conduit/blob/main/conduit.example.toml) and adjust where Conduit will be listening to LLM calls and setup the providers you are using.

Conduit can capture cost values from responses formatted in the common Anthropic (Messages API) and OpenAI (Responses & Chat Completions APIs) or similar responses from other models.

Once a configuration file exists, you can run Conduit in your working directory with the command:

```shell
conduit proxy
```

## Usage

**Once Conduit is running, point your SDK's base URL at it.** The URL is `http://<proxy-address>/<provider_key>` where `<provider_key>` matches a `[providers.<key>]` section in your `conduit.toml`.

Set the base URL environment variable for your provider SDK:

```shell
# NB: OpenAI-like providers also need the `/v1` path suffix:
export OPENAI_BASE_URL=http://localhost:8080/openai/v1
export ANTHROPIC_BASE_URL=http://localhost:8080/anthropic
```

Most SDKs automatically pick up the above env vars with no code changes needed.

Alternatively, pass the base API url directly, e.g. with common Python SDKs:

```python
client = OpenAI(base_url="http://localhost:8080/openai/v1")
client = Anthropic(base_url="http://localhost:8080/anthropic")
```

See [`examples/`](https://github.com/valohai/conduit/tree/main/examples) for complete Python usage examples.

## Development

See the [contributing guide](https://github.com/valohai/conduit?tab=contributing-ov-file) to get started.
