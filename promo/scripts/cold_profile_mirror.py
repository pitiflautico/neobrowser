#!/usr/bin/env python3
"""Cold Profile Mirror — copia un perfil real de Chrome a un perfil NeoBrowser.

Chrome bloquea un user-data-dir exclusivamente (SingletonLock). Para que NeoBrowser
tenga una sesión completa de LinkedIn/GitHub/Reddit/Product Hunt sin inyección de
cookies, el usuario debe cerrar Chrome un momento, ejecutar este script, y luego
puede volver a abrir su Chrome normal. NeoBrowser usará la copia mientras dure.

Uso:
    python3 promo/scripts/cold_profile_mirror.py
    # luego, con Chrome cerrado:
    NEOBROWSER_PROFILE=real python3 promo/scripts/linkedin_post_mcp.py
"""

import os
import shutil
import sys
from pathlib import Path

REAL_PROFILE = os.environ.get("NEOBROWSER_REAL_PROFILE", "Profile 24")
SOURCE = Path.home() / "Library" / "Application Support" / "Google" / "Chrome" / REAL_PROFILE
DEST = Path.home() / ".neobrowser" / "profiles" / "real" / "Default"


def chrome_is_running() -> bool:
    """True si hay procesos Google Chrome (excluyendo helpers de NeoBrowser)."""
    import subprocess

    out = subprocess.run(["ps", "-ax"], capture_output=True, text=True)
    for line in out.stdout.splitlines():
        if "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" in line:
            # NeoBrowser también lanza Chrome, pero desde su propio perfil.
            if "/.neobrowser/profiles/" not in line:
                return True
    return False


def main():
    if chrome_is_running():
        print("ERROR: Chrome real del usuario está corriendo.")
        print("Cierra Chrome completamente (Cmd+Q) y vuelve a ejecutar este script.")
        print("Después podrás volver a abrir Chrome normalmente.")
        sys.exit(1)

    if not SOURCE.exists():
        print(f"ERROR: no existe el perfil real: {SOURCE}")
        print(f"Ajusta NEOBROWSER_REAL_PROFILE o verifica la ruta.")
        sys.exit(1)

    print(f"Copiando {SOURCE} -> {DEST}")
    print("(puede tardar unos minutos si el perfil es grande)")

    if DEST.exists():
        shutil.rmtree(DEST)
    DEST.parent.mkdir(parents=True, exist_ok=True)

    # shutil.copytree conserva metadatos y omite locks/sockets de Chrome.
    def ignore(src, names):
        return {n for n in names if n.startswith("Singleton") or n.endswith(".lock")}

    shutil.copytree(SOURCE, DEST, ignore=ignore)

    print(f"\nPerfil copiado a {DEST}")
    print("Ahora puedes lanzar NeoBrowser contra este perfil:")
    print("  NEOBROWSER_PROFILE=real neobrowser ...")
    print("O publicar en LinkedIn/Reddit con las sesiones reales:")
    print("  NEOBROWSER_PROFILE=real python3 promo/scripts/linkedin_post_mcp.py")
    print("  NEOBROWSER_PROFILE=real python3 promo/scripts/reddit_post_mcp.py")


if __name__ == "__main__":
    main()
