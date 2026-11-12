#!/usr/bin/env bash
# Print 8-ball endpoints (TOML) for every model of a hosted provider that can answer a decision question.
# Usage: HOSTED_URL=https://... HOSTED_API_KEY=... [HOSTED_SKIP=regex] dev/hosted.sh [requests_per_minute [model]] > hosted.toml
# HOSTED_SKIP is a regex for model ids to leave out, for example routers or joke models.
# - decisions models answer through the decisions backend;
# - chat models answer through logprobs, with reasoning turned off; a model whose reasoning
#   cannot be turned off is left out, because the answer must be one token;
# - rerank models, if the provider serves any, use the rerank backend.
# Models that the key cannot use (premium without opt-in) and models that match HOSTED_SKIP are left out.
# Qwen and Gemma turn thinking off through their chat template: a provider's gateway can pass reasoning_effort "none" into the Qwen template, which rejects it, and gemma-4-26b ignores it
# and opens a thought channel first.
set -euo pipefail
per_minute=${1:-30}
only=${2:-}
curl -fsS --retry 3 -H "Authorization: Bearer ${HOSTED_API_KEY:?set HOSTED_API_KEY}" "${HOSTED_URL:?set HOSTED_URL}/v1/models" |
  jq -r --argjson per_minute "$per_minute" --arg only "$only" --arg skip "${HOSTED_SKIP:-}" --arg url "$HOSTED_URL" '
    .data[]
    | select($only == "" or .id == $only)
    | select($skip == "" or (.id | test($skip) | not))
    | select(.premium_unlocked != false)
    | .reasoning = .chat_capabilities.reasoning
    | if (.endpoints | index("decisions")) then .kind = "decisions"
      elif (.endpoints | index("rerank")) then .kind = "rerank"
      elif (.endpoints | index("chat/completions")) then
        if .reasoning.mandatory == true then empty
        elif (.id | test("^(qwen|gemma)"))
        then .kind = "logprob" | .extra = "extra_body = { chat_template_kwargs = { enable_thinking = false } }\n"
        elif .reasoning == null or .reasoning.default_enabled == false then .kind = "logprob"
        elif ((.reasoning.supported_efforts // []) | index("none"))
        then .kind = "logprob" | .extra = "extra_body = { reasoning_effort = \"none\" }\n"
        else empty end
      else empty end
    | "[endpoints.hosted-\(.id | ascii_downcase | gsub("[^a-z0-9]+"; "-"))]\n"
      + "url = \"\($url)\"\nkind = \"\(.kind)\"\nmodel = \"\(.id)\"\n"
      + "api_key_env = \"HOSTED_API_KEY\"\nrequests_per_minute = \($per_minute)\nconcurrency = 1\n"
      + (.extra // "")'
