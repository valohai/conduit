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

# then use OpenAI Chat Completions API as you would normally

stream = client.chat.completions.create(
    model="gpt-5-nano",
    messages=[
        {"role": "user", "content": "Hello! Tell me a fun fact about space."},
    ],
    stream=True,
    stream_options={"include_usage": True},  # !!! must explicitly include usage
)

for chunk in stream:
    if chunk.choices and chunk.choices[0].delta.content:
        print(chunk.choices[0].delta.content, end="", flush=True)

print()
