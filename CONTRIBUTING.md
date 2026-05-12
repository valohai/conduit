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

### Releasing

### Release Tools

```shell
# install cargo-release if not available
cargo install cargo-release --locked
cargo release --version
```

Setup crates.io authentication:

```shell
# if you have logged in to crates.io, registry token should be in:
cat $CARGO_HOME/credentials.toml
# which means the token will be automatically used

# if not, login to crates.io, and possibly request access inside of Valohai
cargo login

# you can some test this via
cargo owner --list valohai-conduit
# valohai (or something else, whoever are the current owners)
```

### Release Step-by-Step

Overview:

1. Use [`cargo-release`](https://github.com/crate-ci/cargo-release) to publish
   a new version on [crates.io](https://crates.io/crates/valohai-conduit) while
   keeping version control in sync.
2. Watch the binary build run
   [in GitHub Actions](https://github.com/valohai/conduit/actions?query=workflow%3ARelease+event%3Apush).
3. After 10 minutes, make sure new release gets announced
   [on GitHub Releases](https://github.com/valohai/conduit/releases).

Confirm that you are on the latest `main` branch:

```shell
git checkout main
git pull
git status
```

Decide on a [bump level](https://github.com/crate-ci/cargo-release/blob/HEAD/docs/reference.md#bump-level):

> usually `patch`, `minor` or `major`

```shell
# this is a _dry run_
cargo release --workspace [level]

# and, if everything looks proper
cargo release --workspace [level] --execute
```

This will:

- update version numbering inside the codebase
- commit the changes
- publish the crates
- tag the commit
- push the tagged commit to remote

A successful `cargo release` ends like this:

```bash
To github.com:valohai/conduit.git
   XXXXXXX..YYYYYYY  main -> main
 * [new tag]         vX.Y.Z -> vX.Y.Z
```

The new git tag will trigger `release` workflow's GitHub Actions
to _build_, _host_ and _announce_ the cross-compiled binaries of
the new version with [dist](https://github.com/axodotdev/cargo-dist).

Keep track how the building goes
[on GitHub Actions](https://github.com/valohai/conduit/actions?query=workflow%3ARelease+event%3Apush).

It will take 10 minutes or so to build all binaries and announce them.

If everything goes as planned,
you will end up with a new release
[on GitHub Releases](https://github.com/valohai/conduit/releases).

Done!
