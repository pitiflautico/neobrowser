"""Unit tests for profile.py cookie sync — no Chrome required."""
import shutil
import sqlite3
import tempfile
from pathlib import Path

import pytest


@pytest.fixture
def profiles(tmp_path):
    src = tmp_path / "src_profile" / "Default"
    dst = tmp_path / "dst_profile" / "Default"
    src.mkdir(parents=True)
    dst.mkdir(parents=True)
    return src.parent, dst.parent


def _create_cookie_db(profile: Path, cookies: list[tuple[str, str]]) -> None:
    """Create a minimal Cookies SQLite DB with given (host, name) pairs."""
    db = profile / "Default" / "Cookies"
    conn = sqlite3.connect(str(db))
    conn.execute("CREATE TABLE cookies (host_key TEXT, name TEXT, value TEXT)")
    conn.executemany("INSERT INTO cookies VALUES (?, ?, '')", cookies)
    conn.commit()
    conn.close()


def _read_hosts(profile: Path) -> set[str]:
    db = profile / "Default" / "Cookies"
    conn = sqlite3.connect(str(db))
    hosts = {row[0] for row in conn.execute("SELECT host_key FROM cookies")}
    conn.close()
    return hosts


def test_sync_copies_cookies(profiles):
    src, dst = profiles
    _create_cookie_db(src, [("example.com", "session"), ("github.com", "token")])

    from chrome.profile import sync_profile
    sync_profile(src, dst)

    hosts = _read_hosts(dst)
    assert "example.com" in hosts
    assert "github.com" in hosts


def test_sync_excludes_google(profiles):
    src, dst = profiles
    _create_cookie_db(src, [
        ("example.com", "session"),
        ("google.com", "SAPISID"),
        (".google.com", "SID"),
        ("accounts.google.com", "oauth"),
        ("gmail.com", "GMAIL_AT"),
    ])

    from chrome.profile import sync_profile
    sync_profile(src, dst)

    hosts = _read_hosts(dst)
    assert "example.com" in hosts
    assert "google.com" not in hosts
    assert ".google.com" not in hosts
    assert "accounts.google.com" not in hosts
    assert "gmail.com" not in hosts


def test_resync_also_excludes_google(profiles):
    """sync_profile and resync use the same function — no divergence."""
    src, dst = profiles
    _create_cookie_db(src, [("google.com", "SID"), ("chatgpt.com", "session")])

    from chrome.profile import sync_profile
    # Call twice (initial sync + resync)
    sync_profile(src, dst)
    sync_profile(src, dst)

    hosts = _read_hosts(dst)
    assert "chatgpt.com" in hosts
    assert "google.com" not in hosts


def test_sync_no_src_cookies_is_noop(profiles):
    src, dst = profiles
    # No Cookies file in src
    from chrome.profile import sync_profile
    sync_profile(src, dst)  # Should not raise
    assert not (dst / "Default" / "Cookies").exists()
