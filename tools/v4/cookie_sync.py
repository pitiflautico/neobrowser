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
import threading
import time
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from tools.v4.chrome_tab import ChromeTab

log = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Paths & constants
# ---------------------------------------------------------------------------

_REAL_CHROME_BASE = (
    Path.home() / "Library" / "Application Support" / "Google" / "Chrome"
)
_SESSIONS_BASE = Path.home() / ".neorender" / "sessions"

REAL_PROFILE = os.environ.get("NEOBROWSER_REAL_PROFILE", "Profile 24")
SYNC_TTL_S = float(os.environ.get("NEOBROWSER_SYNC_TTL", "300"))  # 5 min default

# Google domains to exclude from file-level sync (prevent real Chrome logout)
_EXCLUDED_DOMAINS = (
    ".google.com", ".google.es", ".googleapis.com", ".gstatic.com",
    ".youtube.com", ".accounts.google.com", ".gmail.com",
)

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

        # ── Cookies SQLite ──
        src_cookies = real_profile / "Cookies"
        if src_cookies.exists() and src_cookies.stat().st_size > 0:
            dst_cookies = ghost_default / "Cookies"
            try:
                _copy_with_wal(src_cookies, dst_cookies)
                kept, deleted = _exclude_google_from_db(dst_cookies)
                stats["cookies"] = kept
                stats["deleted"] = deleted
                stats["layers"].append(f"Cookies ({kept} kept, {deleted} Google removed)")
                log.info("cookie_sync: %s cookies synced (%d kept)", profile_name, kept)
            except Exception as exc:
                log.warning("cookie_sync: Cookies sync failed: %s", exc)

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

    # Read cookies from real Chrome SQLite (read-only, no WAL needed for read)
    try:
        conn = sqlite3.connect(f"file:{src_cookies}?mode=ro&nolock=1", uri=True)
        # Columns: host_key, name, value, path, expires_utc, is_secure, is_httponly, ...
        rows = conn.execute(
            "SELECT host_key, name, path, expires_utc, is_secure, is_httponly, samesite, encrypted_value "
            "FROM cookies"
        ).fetchall()
        conn.close()
    except Exception as exc:
        log.warning("cookie_sync: could not read real Chrome cookies: %s", exc)
        return 0

    # Build CDP cookie list (skip Google domains, skip encrypted values)
    cdp_cookies = []
    for host_key, name, path, expires_utc, is_secure, is_httponly, samesite, enc_val in rows:
        if any(host_key.endswith(d) for d in _EXCLUDED_DOMAINS):
            continue
        if not name or enc_val:
            # encrypted_value means we can't read it without Keychain — skip
            # (The file-level copy handles these via macOS Keychain sharing)
            continue
        cdp_cookies.append({
            "name": name,
            "value": "",  # encrypted — can't use this path for value
            "domain": host_key,
            "path": path or "/",
            "secure": bool(is_secure),
            "httpOnly": bool(is_httponly),
        })

    # NOTE: Most session cookies in real Chrome are encrypted via macOS Keychain.
    # inject_from_real_chrome() is a best-effort for non-encrypted cookies.
    # The file-level pre_launch_sync() is the authoritative method — it works
    # because both profiles share the same Keychain decryption key.
    if cdp_cookies:
        try:
            tab.set_cookies(cdp_cookies)
            log.info("cookie_sync: injected %d non-encrypted cookies via CDP", len(cdp_cookies))
        except Exception as exc:
            log.warning("cookie_sync: CDP set_cookies failed: %s", exc)

    # Better path: reload from our saved JSON (which has the decrypted values Chrome gave us)
    return post_launch_restore(tab, profile_name)


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
