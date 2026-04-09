#!/usr/bin/env bash
# V4 MCP launcher — attaches to V3's Chrome if it's already running.
# Reads port from ~/.neorender/neo-browser-port.txt and sets NEOBROWSER_ATTACH_PORT.
# Falls back to standalone mode if the port file is missing or Chrome is not reachable.

PORT_FILE="$HOME/.neorender/neo-browser-port.txt"

if [[ -f "$PORT_FILE" ]]; then
    PORT=$(cat "$PORT_FILE")
    # Quick check: is Chrome actually listening on that port?
    if curl -sf "http://127.0.0.1:${PORT}/json/version" >/dev/null 2>&1; then
        export NEOBROWSER_ATTACH_PORT="$PORT"
    fi
fi

exec python3 "$(dirname "$0")/server.py" "$@"
