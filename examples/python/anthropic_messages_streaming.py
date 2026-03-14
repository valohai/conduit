# /// script
# requires-python = ">=3.14"
# dependencies = ["anthropic"]
# ///

import os

from anthropic import Anthropic

# set `ANTHROPIC_BASE_URL` environment variable or supply `base_url` to client
# that points to the proxy address + [providers.<key>] path from conduit.yaml
base_url = os.environ.get("ANTHROPIC_BASE_URL", "http://localhost:8080/anthropic")
client = Anthropic(base_url=base_url)

# then use Anthropic Messages API as you would normally

with client.messages.stream(
    model="claude-haiku-4-5",
    max_tokens=1024,
    messages=[
        {"role": "user", "content": "Hello! Tell me a fun fact about space."},
    ],
) as stream:
    for text in stream.text_stream:
        print(text, end="", flush=True)

print()
