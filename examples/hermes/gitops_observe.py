#!/usr/bin/env python3
"""Observe a GitOps candidate batch; always wake Hermes with the original snapshot."""

import argparse
from datetime import datetime, timezone
import http.client
import json
import math
import os
from pathlib import Path
import sys
import time
import urllib.error
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        # This fixed API endpoint must never forward credentials through a redirect.
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--observations", required=True, type=Path,
                        help="New private JSONL file; existing files are never overwritten")
    parser.add_argument("--spec", default="gitops-investigate")
    parser.add_argument("--endpoint")
    parser.add_argument("--timeout", type=float, default=25)
    args = parser.parse_args()
    context = {"mode": "observe"}
    try:
        snapshot = json.loads(args.input.read_text(encoding="utf-8"))
        if (not isinstance(snapshot, dict)
                or not isinstance(snapshot.get("context"), str)
                or not isinstance(snapshot.get("items"), list)
                or len(snapshot["items"]) > 64):
            raise ValueError("snapshot requires context text and an items array of at most 64 candidates")
        context["snapshot"] = snapshot
    except (OSError, ValueError) as error:
        context["input_error"] = str(error)
        print(json.dumps({"wakeAgent": True, "context": context}))
        return

    start = time.monotonic()
    result, error = None, None
    try:
        if not math.isfinite(args.timeout) or args.timeout <= 0:
            raise ValueError("timeout must be finite and positive")
        body = {"items": snapshot["items"], "context": snapshot["context"],
                "spec": args.spec, "endpoint": args.endpoint}
        headers = {"Content-Type": "application/json"}
        if key := os.environ.get("EIGHTBALL_API_KEY"):
            headers["Authorization"] = f"Bearer {key}"
        url = os.environ.get("EIGHTBALL_URL", "http://127.0.0.1:8080").rstrip("/")
        request = urllib.request.Request(url + "/v1/triage",
                                         data=json.dumps(body).encode(), headers=headers)
        with urllib.request.build_opener(NoRedirect).open(request, timeout=args.timeout) as response:
            result = json.load(response)
    except (OSError, ValueError, http.client.HTTPException, urllib.error.URLError) as failure:
        error = str(failure)
    record = {"mode": "observe", "observed_at": datetime.now(timezone.utc).isoformat(),
              "spec": args.spec, "endpoint": args.endpoint, "snapshot": snapshot,
              "elapsed_ms": (time.monotonic() - start) * 1000,
              "result": result, "error": error}
    try:
        # One immutable file per run keeps cached decision ids separate from observations.
        descriptor = os.open(args.observations, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            output.write(json.dumps(record, ensure_ascii=False) + "\n")
        context["observations"] = str(args.observations)
    except OSError as failure:
        context["observation_error"] = str(failure)
        print(f"8ball observation could not be saved: {failure}", file=sys.stderr)
    if error:
        context["decision_error"] = error
    # Even confident rejections are only recorded; the pilot never suppresses work.
    print(json.dumps({"wakeAgent": True, "context": context}, ensure_ascii=False))


if __name__ == "__main__":
    main()
