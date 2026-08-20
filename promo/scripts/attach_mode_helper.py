#!/usr/bin/env python3
"""Attach Mode Helper — reinicia Chrome del usuario con --remote-debugging-port.

NeoBrowser puede conectarse a un Chrome ya abierto vía NEOBROWSER_ATTACH_PORT.
Este script cierra Chrome gracefulmente, lo relanza con el puerto de depuración
habilitado y restaura la sesión anterior. Después lanza NeoBrowser en attach mode.

Uso:
    python3 promo/scripts/attach_mode_helper.py
    # o con un puerto custom:
    NEOBROWSER_ATTACH_PORT=9222 python3 promo/scripts/attach_mode_helper.py
"""

import os
import signal
import subprocess
import sys
import time
from pathlib import Path

PORT = int(os.environ.get("NEOBROWSER_ATTACH_PORT", "9222"))
CHROME_BIN = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
REAL_PROFILE = os.environ.get("NEOBROWSER_REAL_PROFILE", "Profile 24")
PROFILE_DIR = Path.home() / "Library" / "Application Support" / "Google" / "Chrome"


def chrome_pids() -> list[int]:
    """PIDs de procesos Google Chrome principales (no helpers de NeoBrowser)."""
    pids = []
    out = subprocess.run(["ps", "-ax", "-o", "pid,command"], capture_output=True, text=True)
    for line in out.stdout.splitlines()[1:]:
        try:
            pid_str, cmd = line.strip().split(None, 1)
        except ValueError:
            continue
        if CHROME_BIN in cmd and "/.neobrowser/profiles/" not in cmd:
            pids.append(int(pid_str))
    return pids


def wait_for_chrome_port(port: int, timeout: float = 30.0) -> bool:
    import urllib.request

    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=1.0) as r:
                if r.status == 200:
                    return True
        except Exception:
            pass
        time.sleep(0.5)
    return False


def main():
    if not Path(CHROME_BIN).exists():
        print(f"ERROR: no encuentro Chrome en {CHROME_BIN}")
        sys.exit(1)

    pids = chrome_pids()
    if pids:
        print(f"Chrome está corriendo con PIDs: {pids}")
        print("Lo reiniciaré con --remote-debugging-port y --restore-last-session.")
        print("Asegúrate de guardar cualquier trabajo importante en pestañas.")
        input("Pulsa Enter para continuar o Ctrl+C para cancelar...")

        for pid in pids:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass

        print("Esperando a que Chrome cierre...")
        deadline = time.time() + 10
        while time.time() < deadline and chrome_pids():
            time.sleep(0.5)

        remaining = chrome_pids()
        if remaining:
            print(f"Forzando cierre de PIDs restantes: {remaining}")
            for pid in remaining:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            time.sleep(1)

    print(f"Relanzando Chrome con --remote-debugging-port={PORT}")
    subprocess.Popen(
        [
            CHROME_BIN,
            f"--remote-debugging-port={PORT}",
            f"--profile-directory={REAL_PROFILE}",
            "--restore-last-session",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    print("Esperando a que Chrome responda en el puerto de depuración...")
    if not wait_for_chrome_port(PORT):
        print("ERROR: Chrome no abrió el puerto de depuración a tiempo.")
        sys.exit(1)

    print(f"Chrome listo. Ahora puedes ejecutar NeoBrowser en attach mode:")
    print(f"  NEOBROWSER_ATTACH_PORT={PORT} neobrowser ...")
    print(f"O publicar con los scripts de promo:")
    print(f"  NEOBROWSER_ATTACH_PORT={PORT} python3 promo/scripts/linkedin_post_mcp.py")


if __name__ == "__main__":
    main()
