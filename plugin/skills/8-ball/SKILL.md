---
name: 8-ball
description: "Use 8-ball MCP to ask a one-off yes/no, category, or rating question, triage a list, or qualify a recurring decision with the user before it filters, routes, or scores real items."
license: MIT
metadata:
  hermes:
    tags: [decision-model, triage, gating]
---

# 8-ball

For a one-off judgment, use the connected MCP directly. For a new filter, routing rule, or rating that people will rely on, follow the [qualification guide](references/qualification.md) with the user. A named spec makes a question reusable. Its name alone says nothing about accuracy on the user's data.

## Ask once

Check the server instructions for available specs. Call `decision_ask` with `{"spec":"greeting","state":"hello there"}` if that spec exists. For an ad-hoc question, supply the question in the call:

```json
{"state":"The last four backups failed.","questions":[{"id":"investigate","type":"noul","instructions":"Does this message report a backup failure?"}]}
```

Ask one judgment per question. Put instructions in the question and the evidence in `state`. Use code for exact rules and permission checks. Use the normal workflow for long documents and text generation.

- Use `noul` for yes/no. For `choice`, give `criteria` as a map of option names to descriptions. For `score`, supply level descriptions in order from lowest to highest. The verdict is a zero-based index.
- Use `decision_triage` for up to 64 items with one `spec` or one inline `question`. Use `keep` for the option or minimum score to keep (default `yes` for noul). Optional shared `context` plus each item must fit the endpoint's state limit.
- On `unsure` or an error, do the normal work. In triage, `rejected` contains judged items. The indices in `prefiltered` were never judged. Review them when a miss matters.
- When a decision requests `report: true`, do the work. Use `decision_label` with the saved decision id and the observed answer if the outcome establishes it. For matters of taste, the user supplies the label.

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
