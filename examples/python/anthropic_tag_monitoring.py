# /// script
# dependencies = ["anthropic"]
# ///
import os
from anthropic import Anthropic

# set `ANTHROPIC_BASE_URL` environment variable or supply `base_url` to client
# that points to the proxy address + [providers.<key>] path from conduit.toml
base_url = os.environ.get("ANTHROPIC_BASE_URL", "http://localhost:8080/anthropic")
client = Anthropic(base_url=base_url)

# Conduit sends tagged requests to Valohai LLM (needs `[valohai-llm]` in conduit.toml),
# where the Monitor page charts their costs per tag value.
# Keys become snake_case, e.g. `X-VH-Tag-Use-Case` is `use_case`; keep tag values few, like teams, not user IDs.
for context in ["support-bot", "summarizer", "batch/nightly"] * 3:
    client.messages.create(
        model="claude-haiku-4-5",
        max_tokens=64,
        messages=[{"role": "user", "content": "Say hi in 3 words."}],
        extra_headers={"X-VH-Tag-Context": context},  # "X-VH-Tag-<key>" defines a tag you can monitor by value
    )
    print("sent", context)
