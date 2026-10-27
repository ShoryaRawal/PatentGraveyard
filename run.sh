#!/bin/sh
# PatentGraveyard launcher.
# The SerpApi key lives here, in the server's environment only — it never
# reaches the browser or the repo (this project is not a git repo; if it ever
# becomes one, uninstall this file from version control before committing).
export SERPAPI_KEY="b44b1ae9cc06e26f513e24c38ff9ba81c41e22f5c387fda193c55a2713edf3fb"
# Groq powers the analyst chat only. Keys stay in the server environment; the
# browser never receives them. Override AGENT_MODEL to change the chat model.
export GROQ_API_KEY="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"
# GraveTender mobile API. The phone sends this as a bearer token. Override it
# in production; the app's Settings screen stores whatever you enter.
export MOBILE_API_TOKEN="${MOBILE_API_TOKEN:-grave-tender-dev}"
# Bind to 0.0.0.0 so the Android Emulator (10.0.2.2) and LAN devices can reach port 3000
export GRAVE_TENDER_ADDR="${GRAVE_TENDER_ADDR:-0.0.0.0:3000}"
# Reports live in SQLite. Override with an absolute path on AWS.
export GRAVE_TENDER_DB="${GRAVE_TENDER_DB:-./gravetender.db}"

cd "$(dirname "$0")"

# No arguments: run the last release build. Otherwise run what you passed,
# e.g. ./run.sh cargo run --no-default-features --features ssr
if [ "$#" -eq 0 ]; then
  set -- ./target/release/patent-graveyard
fi

exec "$@"
