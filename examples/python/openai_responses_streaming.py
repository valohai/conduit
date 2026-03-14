# /// script
# requires-python = ">=3.14"
# dependencies = ["openai"]
# ///

import os

from openai import OpenAI

# set `OPENAI_BASE_URL` environment variable or supply `base_url` to client
# that points to the proxy address + [providers.<key>] path from conduit.yaml
# + /v1 suffix
base_url = os.environ.get("OPENAI_BASE_URL", "http://localhost:8080/openai/v1")
client = OpenAI(base_url=base_url)

# then use OpenAI Responses API as you would normally

with client.responses.stream(
    model="gpt-5-nano",
    input="Hello! Tell me a fun fact about space.",
) as stream:
    for event in stream:
        if event.type == "response.output_text.delta":
            print(event.delta, end="", flush=True)

print()
