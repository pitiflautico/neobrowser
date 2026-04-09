#!/bin/bash
# tools/v4/chrome_launcher.sh
#
# Launch (or reuse) Chrome on a fixed port for NeoBrowser V4.
#
# Usage:
#   ./tools/v4/chrome_launcher.sh          # port 9222, profile "neorender"
#   ./tools/v4/chrome_launcher.sh 9333     # custom port
#   ./tools/v4/chrome_launcher.sh stop     # kill Chrome on default port
#
# Chrome persists cookies, localStorage, and session data across restarts
# because it uses a fixed profile dir: ~/.neorender/profiles/neorender/
#
# If Chrome is already running on the port, this script exits cleanly.

set -euo pipefail

PORT="${1:-9222}"
PROFILE_NAME="neorender"
PROFILE_DIR="${HOME}/.neorender/profiles/${PROFILE_NAME}"
CHROME_BIN="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
PIDFILE="${HOME}/.neorender/chrome-${PORT}.pid"

mkdir -p "${PROFILE_DIR}"
mkdir -p "${HOME}/.neorender"

# ── stop mode ──────────────────────────────────────────────────────────────
if [ "${PORT}" = "stop" ]; then
    PORT="9222"
    if [ -f "${PIDFILE}" ]; then
        PID=$(cat "${PIDFILE}")
        kill "${PID}" 2>/dev/null && echo "Stopped Chrome (pid=${PID})" || echo "Already stopped"
        rm -f "${PIDFILE}"
    else
        echo "No pidfile at ${PIDFILE} — killing by port"
        lsof -ti tcp:"${PORT}" | xargs kill -9 2>/dev/null || true
    fi
    exit 0
fi

# ── health check — already running? ────────────────────────────────────────
if curl -sf --max-time 1 "http://localhost:${PORT}/json/version" >/dev/null 2>&1; then
    echo "Chrome already running on port ${PORT} ✓"
    exit 0
fi

# ── launch ─────────────────────────────────────────────────────────────────
echo "Launching Chrome on port ${PORT} (profile=${PROFILE_NAME})"

"${CHROME_BIN}" \
    --remote-debugging-port="${PORT}" \
    --user-data-dir="${PROFILE_DIR}" \
    --no-first-run \
    --no-default-browser-check \
    --disable-background-networking \
    --disable-sync \
    --disable-translate \
    --disable-extensions \
    --disable-default-apps \
    --metrics-recording-only \
    --safebrowsing-disable-auto-update \
    --password-store=basic \
    --use-mock-keychain \
    2>/dev/null &

CHROME_PID=$!
echo "${CHROME_PID}" > "${PIDFILE}"

# Wait for Chrome to be ready (max 10s)
for i in $(seq 1 20); do
    if curl -sf --max-time 1 "http://localhost:${PORT}/json/version" >/dev/null 2>&1; then
        echo "Chrome ready on port ${PORT} (pid=${CHROME_PID}) ✓"
        exit 0
    fi
    sleep 0.5
done

echo "ERROR: Chrome did not become ready within 10s" >&2
kill "${CHROME_PID}" 2>/dev/null || true
rm -f "${PIDFILE}"
exit 1
