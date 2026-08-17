"""
Tests for the stealth launch configuration (chrome_process).

These lock in the bot-detection fixes: no leaked navigator.webdriver in headless,
no hardcoded/stale User-Agent, no software-WebGL flag, and a UA that matches the
real installed Chrome version.
"""
from neobrowser import chrome_process as cp


def test_headless_flags_suppress_webdriver():
    assert "--disable-blink-features=AutomationControlled" in cp.DEFAULT_CHROME_FLAGS


def test_headless_has_no_hardcoded_user_agent_flag():
    # The UA is applied dynamically at launch (real version), never as a stale
    # constant in the flag list.
    assert not any(f.startswith("--user-agent=") for f in cp.DEFAULT_CHROME_FLAGS)


def test_disable_gpu_not_a_default():
    # --disable-gpu forces SwiftShader (a headless tell); it must be opt-in only.
    assert "--disable-gpu" not in cp.DEFAULT_CHROME_FLAGS


def test_window_size_is_set():
    assert any(f.startswith("--window-size=") for f in cp.DEFAULT_CHROME_FLAGS)


def test_headless_new_mode():
    assert "--headless=new" in cp.DEFAULT_CHROME_FLAGS


def test_visible_flags_also_hide_webdriver():
    assert "--disable-blink-features=AutomationControlled" in cp.VISIBLE_CHROME_FLAGS


def test_user_agent_is_genuine_when_available():
    # On a machine with Chrome installed this returns the real UA; without Chrome
    # it returns None. Either way it must never carry the 'Headless' tell or a
    # stale hardcoded version.
    ua = cp._chrome_user_agent()
    if ua is not None:
        assert "Chrome/" in ua
        assert "Safari/537.36" in ua
        assert "Headless" not in ua
        # It should reflect the real major version, not the old hardcoded 124.
        major = cp._detect_chrome_major(cp.CHROME_BIN)
        if major:
            assert f"Chrome/{major}." in ua


def test_chrome_binary_discovered():
    # Discovery returns a concrete path string (may or may not exist in CI).
    assert isinstance(cp.CHROME_BIN, str) and cp.CHROME_BIN
