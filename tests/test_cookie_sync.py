"""
Tests for cookie_sync fixes: safe cookie injection, garbage-decrypt handling,
real-profile validation, and the duplicate-session exclusion set.
"""
import pytest

from neobrowser import cookie_sync as cs


class _FakeTab:
    """Records send() calls; can be told to fail or time out on specific batches."""

    def __init__(self, fail_on=None, timeout_on=None):
        self.calls = []            # list of batch sizes sent
        self.fail_on = set(fail_on or ())
        self.timeout_on = set(timeout_on or ())

    def send(self, method, params, timeout=None):
        idx = len(self.calls)
        self.calls.append(len(params["cookies"]))
        if idx in self.timeout_on:
            raise TimeoutError("mock timeout")
        if idx in self.fail_on:
            raise RuntimeError("mock failure")
        return {}


def _cookies(n):
    return [{"name": f"c{i}", "value": "x", "domain": ".example.com", "path": "/"} for i in range(n)]


def test_inject_batches_and_counts():
    tab = _FakeTab()
    n = cs._inject_cookies_safely(tab, _cookies(60))
    assert n == 60
    # 60 cookies at batch size 25 -> 25, 25, 10
    assert tab.calls == [25, 25, 10]


def test_inject_aborts_on_timeout():
    # A timeout means the socket is dead — must stop, not hammer every batch.
    tab = _FakeTab(timeout_on={1})
    n = cs._inject_cookies_safely(tab, _cookies(100))
    assert n == 25                 # only the first batch landed
    assert len(tab.calls) == 2     # it stopped after the timeout, didn't try all 4


def test_inject_skips_single_bad_batch():
    # A non-timeout error skips just that batch and continues.
    tab = _FakeTab(fail_on={1})
    n = cs._inject_cookies_safely(tab, _cookies(60))
    assert n == 35                 # 25 + (skip) + 10
    assert tab.calls == [25, 25, 10]


def test_inject_empty_is_noop():
    tab = _FakeTab()
    assert cs._inject_cookies_safely(tab, []) == 0
    assert tab.calls == []


def test_decrypt_rejects_garbage_padding():
    # A too-short / bad-padding plaintext must return None, not a bogus value.
    # _decrypt_chrome_value on a non-v10, non-utf8 blob returns None.
    assert cs._decrypt_chrome_value(b"", b"\x00" * 16) is None
    assert cs._decrypt_chrome_value(b"\xff\xfe\xfd", b"\x00" * 16) is None


def test_real_profile_validation_accepts_real_names():
    for name in ("Profile 24", "Default", "Profile 1"):
        assert cs._validate_real_profile(name) == name


def test_real_profile_validation_rejects_traversal():
    for bad in ("../evil", "a/b", "x\x00y", "z" * 100):
        with pytest.raises(ValueError):
            cs._validate_real_profile(bad)


def test_session_exclusions_cover_the_three_risky_providers():
    keys = set(cs._SESSION_AUTH_EXCLUSIONS.keys())
    assert {"google", "linkedin", "microsoft"} <= keys
    # LinkedIn's session-identity cookie is protected.
    _suffixes, names = cs._SESSION_AUTH_EXCLUSIONS["linkedin"]
    assert "li_at" in names
