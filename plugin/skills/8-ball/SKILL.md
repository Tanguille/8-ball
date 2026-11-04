---
name: 8-ball
description: "Cheap yes/no, pick-one and level judgments from a small decision model through the 8-ball MCP tools (decision_ask, decision_triage), about 0.5 s and no LLM turn. Use BEFORE expensive or noisy work to decide whether it is needed (wake an agent? post this? read these 40 items?), to triage lists, or to bucket things. Not for permissions, exact logic, long documents or generating text."
license: MIT
metadata:
  hermes:
    tags: [decision-model, triage, gating]
---

# 8-ball

The server's MCP instructions carry the full usage rules; follow them. In short: act only on a clear verdict, treat `unsure` and errors as "do the normal thing", one judgment per question, all evidence in `state` (or shared `context` for triage).

## Tools
- `decision_ask`: `{state, spec}` for a named calibrated question, or `{state, questions: [{id, type, instructions, criteria}]}`.
- `decision_triage`: `{items: [...≤64], spec | question, context?, keep?, top_k?}`.

Triage includes `rejected` model decisions and `prefiltered` unjudged indices. Do not treat embedding exclusions as confident rejections. Contextual inputs must fit the endpoint state limit; errors mean continue the normal work.

## From a script
```python
import json, os, urllib.request
req = urllib.request.Request(
    os.environ.get("EIGHTBALL_URL", "http://127.0.0.1:8080") + "/v1/ask",
    data=json.dumps({"spec": "my-spec", "state": text}).encode(),
    headers={"content-type": "application/json",
             "authorization": "Bearer " + os.environ.get("EIGHTBALL_API_KEY", "")})
try:
    verdict = json.load(urllib.request.urlopen(req, timeout=25))["results"][0]["verdict"]
except Exception:
    verdict = "error"   # fail open: treat as "do the work"
```

## Making a question reliable
1. Add it as a spec in the server config (`[specs.<name>]`), reviewed like code.
2. Use it, then label real decisions: `8ball label <id> yes|no|<option>|<level>`.
3. `8ball calibrate <name>` once there are 20+ labelled examples per side. Results then report `calibrated: true`.
