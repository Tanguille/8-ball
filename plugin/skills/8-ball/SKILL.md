---
name: 8-ball
description: "Cheap yes/no, pick-one and level judgments from a small decision model through the 8-ball MCP tools (decision_ask, decision_triage), about 0.5 s and no LLM turn. Use BEFORE expensive or noisy work to decide whether it is needed (wake an agent? post this? read these 40 items?), to triage lists, or to bucket things. Not for permissions, exact logic, long documents or generating text."
license: MIT
metadata:
  hermes:
    tags: [decision-model, triage, gating]
---

# 8-ball

The MCP instructions of the server hold the full usage rules. Follow them. These are the main rules:

- Act only on a clear verdict.
- If the verdict is `unsure` or the call returns an error, do the normal work.
- Ask one judgment per question.
- Put all evidence in `state`, the text that the model judges. For triage, you can put shared evidence in `context`.

## Tools

Call `decision_ask` with `{state, spec}` for a named calibrated question, or with `{state, questions: [{id, type, instructions, criteria}]}`.

Call `decision_triage` with `{items: [...≤64], spec | question, context?, keep?, top_k?}`.

The triage result includes `rejected` model decisions and `prefiltered` unjudged indices. Do not treat embedding exclusions as confident rejections. Contextual inputs must fit the endpoint state limit. If triage returns an error, continue the normal work.

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

1. Add the question as a spec in the server configuration (`[specs.<name>]`). Review the spec like code.
2. Use the spec. Then label real decisions with `8ball label <id> yes|no|<option>|<level>`.
3. If you have 20 or more labeled examples per side, run `8ball calibrate <name>`. Results then report `calibrated: true`.
