#!/usr/bin/env bash
# NeoBrowser MCP launcher.
#
# Standalone by default: launches and owns its own headless Chrome.
#
# Optional attach mode: if NEOBROWSER_ATTACH_PORT is already set, or a running
# Chrome exposes a remote-debugging port recorded in ~/.neorender/neo-browser-port.txt,
# NeoBrowser attaches to that Chrome instead of launching its own. This lets it
# reuse an already-authenticated browser session.

PORT_FILE="$HOME/.neorender/neo-browser-port.txt"

# Only auto-discover a port if the caller didn't set one explicitly.
if [[ -z "$NEOBROWSER_ATTACH_PORT" && -f "$PORT_FILE" ]]; then
    PORT=$(cat "$PORT_FILE")
    if curl -sf "http://127.0.0.1:${PORT}/json/version" >/dev/null 2>&1; then
        export NEOBROWSER_ATTACH_PORT="$PORT"
    fi
fi

exec python3 "$(dirname "$0")/server.py" "$@"
