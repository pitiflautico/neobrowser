"""
Regression guard for the headless-repaint fix.

Under --headless=new the compositor is idle until a frame is forced, so
requestAnimationFrame / IntersectionObserver / virtualized lists never render.
The fix has three parts: anti-throttle launch flags, focus emulation at tab
setup, and ChromeTab.nudge_frame() to force frames on demand. These tests lock
in the parts that can be checked without a live browser so they are not silently
removed.
"""
from neobrowser.chrome_process import DEFAULT_CHROME_FLAGS
from neobrowser.chrome_tab import ChromeTab


def test_anti_throttle_flags_present():
    for flag in (
        "--disable-backgrounding-occluded-windows",
        "--disable-renderer-backgrounding",
        "--disable-background-timer-throttling",
    ):
        assert flag in DEFAULT_CHROME_FLAGS, f"missing anti-throttle flag: {flag}"


def test_nudge_frame_is_defined_and_callable():
    assert hasattr(ChromeTab, "nudge_frame")
    assert callable(ChromeTab.nudge_frame)


def test_nudge_frame_captures_tiny_clip_and_never_raises():
    """nudge_frame issues clipped captureScreenshot calls and swallows failures."""
    calls = []

    class FakeTab:
        nudge_frame = ChromeTab.nudge_frame

        def send(self, method, params=None, timeout=None):
            calls.append((method, params))
            return {}

    ft = FakeTab()
    ft.nudge_frame(count=2)
    assert calls, "nudge_frame issued no CDP calls"
    method, params = calls[0]
    assert method == "Page.captureScreenshot"
    assert params["clip"] == {"x": 0, "y": 0, "width": 1, "height": 1, "scale": 1}
    assert params["format"] == "jpeg"


def test_nudge_frame_stops_on_error():
    """If a frame call raises, nudge_frame breaks instead of looping/raising."""
    class BoomTab:
        nudge_frame = ChromeTab.nudge_frame

        def send(self, method, params=None, timeout=None):
            raise RuntimeError("socket dead")

    BoomTab().nudge_frame(count=3)  # must not raise
