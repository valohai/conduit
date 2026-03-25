# Conduit - How to Run It Locally

This guide explains the quickest way to run Conduit locally for OpenAI or Anthropic testing.

## What Conduit is

Conduit is a local proxy plus dashboard for LLM traffic.

You run the proxy locally, point your OpenAI or Anthropic client to it instead of the provider endpoint directly, and Conduit records the requests so you can inspect them in the dashboard.

## What you need

- Rust toolchain installed
- Cargo available in your shell
- `mise` installed if you want to match the repo's pinned local tooling
- An OpenAI or Anthropic API key
- Python if you want to run the example Python scripts

## Repo structure

At the top level, the repository includes:

- `Cargo.toml` and `Cargo.lock` - Rust workspace metadata
- `rust-toolchain.toml` - pinned Rust toolchain
- `mise.toml` - local dev tooling setup
- `conduit.example.toml` - example config file
- `crates/` - Rust crates in the workspace
- `examples/` - usage examples

## Quick start

### 1. Install tooling

macOS with Homebrew:

```bash
brew install mise
```

Install the Rust toolchain if needed:

```bash
curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"
```

Install the repo-specific tool versions:

```bash
mise install
```

### 2. Copy the config file

From the repo root:

```bash
cp conduit.example.toml conduit.toml
```

For basic OpenAI or Anthropic testing, the default config should be enough.

### 3. Start the proxy

In terminal 1:

```bash
cargo run -- proxy
```

Expected output should look similar to:

```text
proxy running at http://127.0.0.1:8080
```

### 4. Start the dashboard

In terminal 2:

```bash
cargo run -- dashboard
```

This opens the local dashboard that shows recorded requests.

### 5. Set the provider base URL to the proxy

In terminal 3, export your API key and point the SDK to Conduit.

For OpenAI:

```bash
export OPENAI_API_KEY="your_openai_api_key"
export OPENAI_BASE_URL="http://localhost:8080/openai/v1"
```

For Anthropic, use the matching Conduit path from the config and examples.

### 6. Run an example client script

Example:

```bash
python3 openai_test.py
```

If your script uses the OpenAI SDK and picks up `OPENAI_BASE_URL`, it should send traffic through Conduit instead of directly to OpenAI.

## Minimal OpenAI test script

```python
import os
from openai import OpenAI

base_url = os.environ.get("OPENAI_BASE_URL", "http://localhost:8080/openai/v1")
client = OpenAI(base_url=base_url)

stream = client.completions.create(
    model="gpt-3.5-turbo-instruct",
    prompt="Hello! Tell me a fun fact about space.",
    max_tokens=256,
    stream=True,
    stream_options={"include_usage": True},
)

for chunk in stream:
    if chunk.choices:
        text = chunk.choices[0].text
        if text:
            print(text, end="", flush=True)

print()
```

## What success looks like

If everything is working:

- the proxy terminal shows Conduit is running on localhost
- the Python script returns a valid model response
- the dashboard shows the request
- the dashboard records the provider, model, timing, and token counts

## How to use Conduit

The basic workflow is:

1. Run `cargo run -- proxy`
2. Run `cargo run -- dashboard`
3. Point your provider SDK base URL to Conduit on localhost
4. Run your application normally
5. Inspect the recorded calls in the dashboard

You do not need to rewrite your LLM logic. The key change is routing traffic through the local proxy.

## What Conduit is good for

Conduit is useful for:

- local debugging of LLM traffic
- seeing what requests are actually going out
- inspecting model usage and token consumption
- getting a lightweight observability layer without changing much application code
- acting as a local collector before this kind of data is sent to a future backend such as llm.valohai.com

## Common pitfalls

### Nothing appears in the dashboard

Usually one of these:

- the proxy is not running
- `OPENAI_BASE_URL` or `ANTHROPIC_BASE_URL` is not set correctly
- the path is wrong and does not match the Conduit provider route

### Authentication errors

Check that your provider API key is exported in the same terminal where you run the client script.

### Connection refused

The proxy is not running, or it is running on a different port.

### Script works but bypasses Conduit

Your SDK may still be using the provider's default URL. Verify the environment variable is being picked up or pass the `base_url` explicitly in code.

## Recommended terminal layout

- Terminal 1: `cargo run -- proxy`
- Terminal 2: `cargo run -- dashboard`
- Terminal 3: your Python or app script

## Suggested next tests

Once the basic setup works, try:

- multiple requests in a row
- a longer prompt
- a different model
- an invalid model name to see how failures appear
- your own small app instead of the sample script

## Suggested sample repos to try next

A few good follow-up projects to route through Conduit:

- Valohai RAG example: https://github.com/valohai/rag-doc-example
- OpenAI Python examples: https://github.com/openai/openai-python/tree/main/examples
- Anthropic Python SDK examples: https://github.com/anthropics/anthropic-sdk-python/tree/main/examples
- Vercel AI chatbot example: https://github.com/vercel/ai-chatbot
- LangChain templates: https://github.com/langchain-ai/langchain/tree/master/templates

## One-line summary

Conduit is a simple way to capture your LLM activity and experiments: you keep your code mostly the same, route requests through the local proxy, and the basic observability gets recorded automatically.
