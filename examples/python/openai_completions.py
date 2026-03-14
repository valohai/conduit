# /// script
# requires-python = ">=3.14"
# dependencies = ["openai"]
# ///

import os

from openai import OpenAI

# NB: OpenAI Completions API endpoint received its final update in July 2023.
#     You should migrate to OpenAI Responses or Chat Completions API.

# set `OPENAI_BASE_URL` environment variable or supply `base_url` to client
# that points to the proxy address + [providers.<key>] path from conduit.yaml
# + /v1 suffix
base_url = os.environ.get("OPENAI_BASE_URL", "http://localhost:8080/openai/v1")
client = OpenAI(base_url=base_url)

# then use OpenAI Completions API as you would normally

response = client.completions.create(
    model="gpt-3.5-turbo-instruct",
    prompt="Hello! Tell me a fun fact about space.",
    max_tokens=256,
)

print(response.choices[0].text)
