"""
tools/v4/cookie_sync.py

Three-layer session persistence for V4.

Layer 1 — Pre-launch file sync (SQLite + Local Storage + IndexedDB)
    Copy from the real Chrome profile to the ghost profile dir BEFORE
    ChromeProcess.launch(). Chrome reads them natively at startup.
    Source: ~/Library/Application Support/Google/Chrome/{REAL_PROFILE}/
    Dest:   ~/.neorender/profiles/{name}/Default/

Layer 2 — Post-launch CDP inject (JSON → Network.setCookies)
    After Chrome starts, inject persisted cookies into the running tab
    via CDP. No Chrome restart needed. Uses the JSON session cache.
    Source: ~/.neorender/sessions/{name}/cookies.json
    Target: running ChromeTab via Network.setCookies

Layer 3 — Auto-save (tab → JSON session cache)
    Save all live tab cookies + localStorage to the session cache.
    Called explicitly via save_session(tab, profile_name).
    Source: running ChromeTab
    Dest:   ~/.neorender/sessions/{name}/{cookies,local_storage,manifest}.json

Excluded domains (Google):
    Google detects duplicate sessions from headless Chrome and logs out
    the real browser. These domains are never synced file-to-file.
    CDP injection: excluded only during pre-launch; fine to save/restore
    post-launch because they come from the tab's own session.

Environment:
    NEOBROWSER_REAL_PROFILE  — Chrome profile subfolder name (default: "Profile 24")
    NEOBROWSER_SYNC_TTL      — seconds between file syncs (default: 300)
"""
from __future__ import annotations

import json
import logging
import os
import shutil
import sqlite3
import subprocess
import threading
import time
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from neobrowser.chrome_tab import ChromeTab

log = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# macOS Chrome cookie decryption (AES-128-CBC via Keychain)
# ---------------------------------------------------------------------------

def _get_chrome_decrypt_key() -> bytes | None:
    """Retrieve Chrome Safe Storage password from macOS Keychain and derive AES key."""
    try:
        import hashlib
        result = subprocess.run(
            ["security", "find-generic-password", "-w", "-s", "Chrome Safe Storage", "-a", "Chrome"],
            capture_output=True, text=True, timeout=5,
        )
        if result.returncode != 0:
            log.warning("cookie_sync: Keychain lookup failed: %s", result.stderr.strip())
            return None
        password = result.stdout.strip().encode("utf-8")
        # PBKDF2-HMAC-SHA1: same params Chrome uses on macOS
        key = hashlib.pbkdf2_hmac("sha1", password, b"saltysalt", 1003, dklen=16)
        return key
    except Exception as exc:
        log.warning("cookie_sync: could not derive Chrome decrypt key: %s", exc)
        return None


def _decrypt_chrome_value(encrypted: bytes, key: bytes) -> str | None:
    """
    Decrypt a Chrome cookie value encrypted with AES-128-CBC.

    Format (macOS Chrome v10):
      - 3 bytes prefix: 'v10'
      - ciphertext (AES-128-CBC, IV = 16 space chars)
      - plaintext has a 32-byte internal prefix Chrome prepends before encrypting

    Returns plaintext string or None on failure.
    """
    if not encrypted:
        return None
    if encrypted[:3] == b"v10":
        ciphertext = encrypted[3:]
    else:
        # Unencrypted legacy value
        try:
            return encrypted.decode("utf-8")
        except Exception:
            return None
    try:
        from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
        from cryptography.hazmat.backends import default_backend
        iv = b" " * 16
        cipher = Cipher(algorithms.AES(key), modes.CBC(iv), backend=default_backend())
        decryptor = cipher.decryptor()
        plaintext = decryptor.update(ciphertext) + decryptor.finalize()
        # PKCS7 unpad
        pad_len = plaintext[-1]
        unpadded = plaintext[:-pad_len]
        # Skip the 32-byte internal prefix Chrome prepends to every plaintext
        return unpadded[32:].decode("utf-8", errors="replace")
    except Exception as exc:
        log.debug("cookie_sync: decrypt failed for value: %s", exc)
        return None


def decrypt_real_chrome_cookies(domains: list[str] | None = None) -> list[dict]:
    """
    Read and decrypt all cookies from the real Chrome profile.

    Uses macOS Keychain to get the AES key — no SQLite copy needed.
    Returns a list of CDP-ready cookie dicts (name, value, domain, path,
    secure, httpOnly, expires). Skips cookies that fail to decrypt.

    Args:
        domains: if given, only return cookies whose host_key ends with one
                 of these strings (e.g. ['.linkedin.com']).
    """
    key = _get_chrome_decrypt_key()
    if key is None:
        log.warning("cookie_sync: skipping decrypt_real_chrome_cookies (no key)")
        return []

    src = _REAL_CHROME_BASE / REAL_PROFILE / "Cookies"
    if not src.exists():
        log.warning("cookie_sync: real Chrome Cookies not found: %s", src)
        return []

    try:
        # Open read-only without WAL lock
        conn = sqlite3.connect(f"file:{src}?mode=ro&nolock=1", uri=True)
        rows = conn.execute(
            "SELECT host_key, name, value, path, expires_utc, is_secure, "
            "is_httponly, samesite, encrypted_value FROM cookies"
        ).fetchall()
        conn.close()
    except Exception as exc:
        log.warning("cookie_sync: could not read real Chrome Cookies: %s", exc)
        return []

    _google_domains = _FILE_SYNC_EXCLUDED_DOMAINS  # shorthand

    cdp_cookies: list[dict] = []
    skipped = 0
    for host_key, name, value, path, expires_utc, is_secure, is_httponly, samesite, enc_val in rows:
        if not name:
            continue
        if domains and not any(host_key.endswith(d) for d in domains):
            continue
        # For Google domains: allow preference/search cookies, block auth session cookies
        is_google = any(host_key.endswith(d) for d in _google_domains)
        if is_google and name in _GOOGLE_AUTH_COOKIE_NAMES:
            continue

        # Prefer decrypted value
        if enc_val:
            decoded = _decrypt_chrome_value(enc_val, key)
            if decoded is None:
                skipped += 1
                continue
            cookie_value = decoded
        else:
            cookie_value = value or ""

        # Convert Chrome epoch (microseconds since 1601-01-01) to Unix timestamp
        expires_unix = 0
        if expires_utc and expires_utc > 0:
            expires_unix = int((expires_utc - 11644473600_000_000) / 1_000_000)

        samesite_map = {0: "Strict", 1: "Lax", 2: "None", -1: "Unspecified"}
        cdp_cookies.append({
            "name": name,
            "value": cookie_value,
            "domain": host_key,
            "path": path or "/",
            "secure": bool(is_secure),
            "httpOnly": bool(is_httponly),
            "sameSite": samesite_map.get(samesite, "Lax"),
            **({"expires": expires_unix} if expires_unix > 0 else {}),
        })

    log.info(
        "cookie_sync: decrypted %d cookies from real Chrome (%d skipped)",
        len(cdp_cookies), skipped,
    )
    return cdp_cookies

# ---------------------------------------------------------------------------
# Paths & constants
# ---------------------------------------------------------------------------

_REAL_CHROME_BASE = (
    Path.home() / "Library" / "Application Support" / "Google" / "Chrome"
)
_SESSIONS_BASE = Path.home() / ".neorender" / "sessions"

REAL_PROFILE = os.environ.get("NEOBROWSER_REAL_PROFILE", "Profile 24")
SYNC_TTL_S = float(os.environ.get("NEOBROWSER_SYNC_TTL", "300"))  # 5 min default

# Domains excluded from SQLite file-level sync (legacy path, kept for _exclude_google_from_db).
# File-level copy is no longer used (replaced by Keychain decrypt + CDP inject),
# but the list is still used if someone calls _exclude_google_from_db directly.
_FILE_SYNC_EXCLUDED_DOMAINS = (
    ".google.com", ".google.es", ".googleapis.com", ".gstatic.com",
    ".youtube.com", ".accounts.google.com", ".gmail.com",
)

# Alias used by legacy code paths that reference _EXCLUDED_DOMAINS directly.
_EXCLUDED_DOMAINS = _FILE_SYNC_EXCLUDED_DOMAINS

# Google auth cookie names to skip during CDP inject.
# These session-identity cookies could cause Google to flag a duplicate login.
# Preference/consent/search cookies (NID, CONSENT, AEC, SOCS, ANID…) are safe to inject.
_GOOGLE_AUTH_COOKIE_NAMES = frozenset({
    "SID", "HSID", "SSID", "APISID", "SAPISID",
    "__Secure-1PSID", "__Secure-3PSID",
    "__Secure-1PAPISID", "__Secure-3PAPISID",
    "__Secure-1PSIDCC", "__Secure-3PSIDCC",
    "__Secure-1PSIDTS", "__Secure-3PSIDTS",
    "SIDCC", "LSID",
})

_sync_lock = threading.Lock()  # one sync at a time across all threads


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

def _session_dir(profile_name: str) -> Path:
    d = _SESSIONS_BASE / profile_name
    d.mkdir(parents=True, exist_ok=True)
    return d


def _manifest_path(profile_name: str) -> Path:
    return _session_dir(profile_name) / "manifest.json"


def _read_manifest(profile_name: str) -> dict:
    p = _manifest_path(profile_name)
    if not p.exists():
        return {}
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except Exception:
        return {}


def _write_manifest(profile_name: str, data: dict) -> None:
    p = _manifest_path(profile_name)
    p.write_text(json.dumps(data, indent=2), encoding="utf-8")
    p.chmod(0o600)


def _copy_with_wal(src: Path, dst: Path) -> None:
    """
    Copy a SQLite DB file + its WAL/SHM companions.

    Raw file copy preserves uncommitted WAL writes that sqlite3.backup()
    misses. We then checkpoint the copy so the ghost Chrome gets a clean
    single-file DB with all the latest data.
    """
    shutil.copy2(src, dst)
    for suffix in ("-wal", "-shm"):
        src_aux = src.parent / (src.name + suffix)
        dst_aux = dst.parent / (dst.name + suffix)
        if src_aux.exists():
            shutil.copy2(src_aux, dst_aux)
        elif dst_aux.exists():
            try:
                dst_aux.unlink()
            except OSError:
                pass
    # Checkpoint: merge WAL into main DB so Chrome gets a clean single-file DB
    try:
        conn = sqlite3.connect(str(dst))
        conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        conn.commit()
        conn.close()
    except Exception as exc:
        log.debug("WAL checkpoint failed (non-fatal): %s", exc)


def _exclude_google_from_db(cookies_db: Path) -> tuple[int, int]:
    """
    Remove Google-domain cookies from a SQLite cookies DB in-place.
    Returns (kept, deleted).
    """
    try:
        conn = sqlite3.connect(str(cookies_db))
        excluded = " OR ".join("host_key LIKE ?" for _ in _EXCLUDED_DOMAINS)
        params = [f"%{d}" for d in _EXCLUDED_DOMAINS]
        deleted = conn.execute(f"DELETE FROM cookies WHERE {excluded}", params).rowcount
        count = conn.execute("SELECT COUNT(*) FROM cookies").fetchone()[0]
        conn.commit()
        conn.close()
        return count, deleted
    except Exception as exc:
        log.warning("Could not filter Google cookies: %s", exc)
        return 0, 0


# ---------------------------------------------------------------------------
# Layer 1: Pre-launch file sync
# ---------------------------------------------------------------------------

def pre_launch_sync(profile_dir: Path, profile_name: str, force: bool = False) -> dict:
    """
    Copy cookies + storage from the real Chrome profile to the ghost profile dir.

    Called BEFORE ChromeProcess.launch() so Chrome reads them natively.
    Skips if last sync was within SYNC_TTL_S (unless force=True).

    Returns a stats dict: {synced: bool, cookies: int, deleted: int, layers: [...]}
    """
    with _sync_lock:
        manifest = _read_manifest(profile_name)
        last_sync = manifest.get("synced_at", 0)
        if not force and (time.time() - last_sync) < SYNC_TTL_S:
            log.debug(
                "cookie_sync: skipping pre-launch sync for %s (%.0fs ago, TTL=%.0fs)",
                profile_name, time.time() - last_sync, SYNC_TTL_S,
            )
            return {"synced": False, "reason": "ttl_not_expired"}

        real_profile = _REAL_CHROME_BASE / REAL_PROFILE
        if not real_profile.exists():
            log.warning("cookie_sync: real Chrome profile not found: %s", real_profile)
            return {"synced": False, "reason": "real_profile_not_found"}

        # Ghost Chrome stores its data under profile_dir/Default/
        ghost_default = profile_dir / "Default"
        ghost_default.mkdir(parents=True, exist_ok=True)

        stats: dict = {"synced": True, "layers": []}

        # ── Cookies: decrypt from real Chrome → JSON (no SQLite copy) ──
        # We never copy the Cookies SQLite file. Copying it would make the ghost
        # Chrome use the same session tokens as the real Chrome, causing sites
        # (LinkedIn, etc.) to detect a duplicate session and log out the user.
        # Instead, we decrypt cookies via macOS Keychain and persist them as JSON.
        # post_launch_restore() injects them via CDP (in-memory only, no file clash).
        try:
            cdp_cookies = decrypt_real_chrome_cookies()
            if cdp_cookies:
                cookies_path = _session_dir(profile_name) / "cookies.json"
                cookies_path.write_text(json.dumps(cdp_cookies, indent=2), encoding="utf-8")
                cookies_path.chmod(0o600)
                stats["cookies"] = len(cdp_cookies)
                stats["layers"].append(f"Cookies ({len(cdp_cookies)} decrypted via Keychain)")
                log.info("cookie_sync: %d cookies decrypted for %s", len(cdp_cookies), profile_name)
            else:
                stats["cookies"] = 0
        except Exception as exc:
            log.warning("cookie_sync: Cookies decrypt failed: %s", exc)

        # ── Local Storage ──
        for storage_dir in ("Local Storage", "Session Storage"):
            src = real_profile / storage_dir
            dst = ghost_default / storage_dir
            if src.exists():
                try:
                    if dst.exists():
                        shutil.rmtree(dst)
                    shutil.copytree(src, dst)
                    stats["layers"].append(storage_dir)
                except Exception as exc:
                    log.warning("cookie_sync: %s sync failed: %s", storage_dir, exc)

        # ── IndexedDB ──
        src_idb = real_profile / "IndexedDB"
        dst_idb = ghost_default / "IndexedDB"
        if src_idb.exists():
            try:
                if dst_idb.exists():
                    shutil.rmtree(dst_idb)
                shutil.copytree(src_idb, dst_idb)
                stats["layers"].append("IndexedDB")
            except Exception as exc:
                log.warning("cookie_sync: IndexedDB sync failed: %s", exc)

        # ── Update manifest ──
        manifest.update({
            "synced_at": time.time(),
            "profile": profile_name,
            "real_profile": str(real_profile),
            "layers": stats["layers"],
            "cookie_count": stats.get("cookies", 0),
        })
        _write_manifest(profile_name, manifest)
        log.info("cookie_sync: pre-launch sync complete for %s: %s", profile_name, stats["layers"])
        return stats


# ---------------------------------------------------------------------------
# Layer 2: Post-launch CDP inject
# ---------------------------------------------------------------------------

def post_launch_restore(tab: "ChromeTab", profile_name: str) -> int:
    """
    Inject persisted cookies from the JSON session cache into a running tab.

    Called immediately after ChromeTab.open() — no Chrome restart needed.
    Returns number of cookies injected. Returns 0 if no cache exists.
    """
    cookies_path = _session_dir(profile_name) / "cookies.json"
    if not cookies_path.exists():
        log.debug("cookie_sync: no JSON session cache for %s", profile_name)
        return 0
    try:
        cookies = json.loads(cookies_path.read_text(encoding="utf-8"))
        if not cookies:
            return 0
        tab.set_cookies(cookies)
        log.info("cookie_sync: injected %d cookies into tab for %s", len(cookies), profile_name)
        return len(cookies)
    except Exception as exc:
        log.warning("cookie_sync: CDP inject failed for %s: %s", profile_name, exc)
        return 0


def inject_from_real_chrome(tab: "ChromeTab", profile_name: str) -> int:
    """
    Re-sync cookies from real Chrome SQLite → inject into running tab via CDP.

    Used when a login wall is detected after Chrome is already running.
    Does NOT require restarting Chrome — updates the in-memory cookie store.
    Returns number of cookies injected.
    """
    real_profile = _REAL_CHROME_BASE / REAL_PROFILE
    src_cookies = real_profile / "Cookies"
    if not src_cookies.exists():
        log.warning("cookie_sync: real Chrome Cookies not found for re-sync")
        return 0

    # Decrypt all cookies from the real Chrome Keychain and inject via CDP
    cdp_cookies = decrypt_real_chrome_cookies()
    if not cdp_cookies:
        log.warning("cookie_sync: inject_from_real_chrome: no cookies decrypted")
        return 0

    # Save to JSON so post_launch_restore can use them on next startup
    try:
        cookies_path = _session_dir(profile_name) / "cookies.json"
        cookies_path.write_text(json.dumps(cdp_cookies, indent=2), encoding="utf-8")
        cookies_path.chmod(0o600)
    except Exception as exc:
        log.warning("cookie_sync: could not persist cookies JSON: %s", exc)

    try:
        tab.set_cookies(cdp_cookies)
        log.info("cookie_sync: injected %d decrypted cookies via CDP for %s", len(cdp_cookies), profile_name)
        return len(cdp_cookies)
    except Exception as exc:
        log.warning("cookie_sync: CDP set_cookies failed: %s", exc)
        return 0


# ---------------------------------------------------------------------------
# Layer 3: Auto-save (tab → JSON session cache)
# ---------------------------------------------------------------------------

def save_session(tab: "ChromeTab", profile_name: str) -> dict:
    """
    Save all live tab cookies + localStorage to the JSON session cache.

    Cookies are retrieved via CDP (already decrypted by Chrome) and saved
    as JSON. On next V4 startup, post_launch_restore() injects them back
    without needing the real Chrome profile to be present.

    Returns stats dict: {cookies: int, domains: [...], saved_at: float}
    """
    session_dir = _session_dir(profile_name)
    stats: dict = {}

    # ── Cookies ──
    try:
        cookies = tab.get_cookies()
        cookies_path = session_dir / "cookies.json"
        cookies_path.write_text(json.dumps(cookies, indent=2), encoding="utf-8")
        cookies_path.chmod(0o600)
        domains = list({c.get("domain", "") for c in cookies})
        stats["cookies"] = len(cookies)
        stats["domains"] = domains
        log.info(
            "cookie_sync: saved %d cookies for %s (%d domains)",
            len(cookies), profile_name, len(domains),
        )
    except Exception as exc:
        log.warning("cookie_sync: failed to save cookies: %s", exc)
        stats["cookies"] = 0
        stats["domains"] = []

    # ── localStorage (best-effort via JS) ──
    try:
        ls_raw = tab.js("""
            const out = {};
            try {
                for (let i = 0; i < localStorage.length; i++) {
                    const k = localStorage.key(i);
                    out[k] = localStorage.getItem(k);
                }
            } catch(e) {}
            return JSON.stringify({url: location.href, data: out});
        """)
        if ls_raw:
            ls_data = json.loads(ls_raw)
            ls_path = session_dir / "local_storage.json"
            existing = {}
            if ls_path.exists():
                try:
                    existing = json.loads(ls_path.read_text(encoding="utf-8"))
                except Exception:
                    pass
            url = ls_data.get("url", "unknown")
            existing[url] = ls_data.get("data", {})
            ls_path.write_text(json.dumps(existing, indent=2), encoding="utf-8")
            ls_path.chmod(0o600)
            stats["local_storage_keys"] = len(ls_data.get("data", {}))
    except Exception as exc:
        log.debug("cookie_sync: localStorage save failed (non-fatal): %s", exc)

    # ── Manifest ──
    stats["saved_at"] = time.time()
    stats["profile"] = profile_name
    manifest = _read_manifest(profile_name)
    manifest.update({
        "saved_at": stats["saved_at"],
        "cookie_count": stats.get("cookies", 0),
        "domains": stats.get("domains", []),
    })
    _write_manifest(profile_name, manifest)

    return stats


def restore_local_storage(tab: "ChromeTab", profile_name: str) -> int:
    """
    Inject saved localStorage into the current page via JS.
    Returns number of keys restored. Must be called after navigating to the target URL.
    """
    ls_path = _session_dir(profile_name) / "local_storage.json"
    if not ls_path.exists():
        return 0
    try:
        all_ls = json.loads(ls_path.read_text(encoding="utf-8"))
        current_url = tab.current_url() or ""
        # Find best matching entry (exact URL or same origin)
        data = all_ls.get(current_url)
        if data is None:
            # Try origin match
            from urllib.parse import urlparse
            try:
                origin = urlparse(current_url).netloc
                for url, d in all_ls.items():
                    if urlparse(url).netloc == origin:
                        data = d
                        break
            except Exception:
                pass
        if not data:
            return 0
        # Inject via JS
        js_entries = json.dumps(data)
        injected = tab.js(f"""
            const data = {js_entries};
            let count = 0;
            try {{
                for (const [k, v] of Object.entries(data)) {{
                    localStorage.setItem(k, v);
                    count++;
                }}
            }} catch(e) {{}}
            return count;
        """)
        count = int(injected or 0)
        log.info("cookie_sync: restored %d localStorage keys for %s", count, profile_name)
        return count
    except Exception as exc:
        log.debug("cookie_sync: localStorage restore failed (non-fatal): %s", exc)
        return 0


# ---------------------------------------------------------------------------
# Convenience: session info
# ---------------------------------------------------------------------------

def session_info(profile_name: str) -> dict:
    """Return manifest + file existence for a profile. Useful for debugging."""
    manifest = _read_manifest(profile_name)
    session_dir = _session_dir(profile_name)
    return {
        "profile": profile_name,
        "manifest": manifest,
        "cookies_json_exists": (session_dir / "cookies.json").exists(),
        "local_storage_json_exists": (session_dir / "local_storage.json").exists(),
        "real_profile": str(_REAL_CHROME_BASE / REAL_PROFILE),
        "real_profile_exists": (_REAL_CHROME_BASE / REAL_PROFILE).exists(),
        "sync_ttl_s": SYNC_TTL_S,
    }
