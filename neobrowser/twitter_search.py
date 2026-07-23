"""
tools/v4/twitter_search.py

Twitter/X video search for NeoBrowser V4.

Searches Twitter via https://x.com/search for tweets containing video.
Uses its OWN visible Chrome instance (not headless) to avoid bot detection.
The visible Chrome is launched once and reused across calls.

Public function:
    search_twitter_videos(query, count) → list[TweetVideoResult]

Note: does NOT take a `tab` argument — manages its own browser lifecycle.
This is intentional: Twitter requires visible Chrome with real user session.

TweetVideoResult
----------------
  author       str   — @handle of the tweeter
  display_name str   — display name
  text         str   — tweet text (first 300 chars)
  url          str   — direct link to the tweet
  timestamp    str   — ISO datetime or relative
  metrics      dict  — {replies, reposts, likes, views} as strings
  download_cmd str   — yt-dlp command to download the video
"""
from __future__ import annotations

import json
import logging
import re
import time
import urllib.parse
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from neobrowser.chrome_tab import ChromeTab

log = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Visible browser singleton
# ---------------------------------------------------------------------------

_PORT_FILE = Path.home() / ".neorender" / "twitter-chrome-port.txt"
_visible_tab: "ChromeTab | None" = None
_visible_pid: int | None = None


def _get_visible_tab() -> "ChromeTab":
    """
    Get or create a visible Chrome tab for Twitter.

    Strategy:
    1. Reuse existing tab if alive.
    2. Check port file for a previously launched visible Chrome.
    3. Check NEOBROWSER_ATTACH_PORT (user's real Chrome).
    4. Scan for any running neorender Chrome with a real session.
    5. Launch a new visible Chrome (user must login once).
    """
    global _visible_tab, _visible_pid

    # 1. Reuse existing tab
    if _visible_tab is not None:
        try:
            _visible_tab.js("1+1")  # health check
            return _visible_tab
        except Exception:
            _visible_tab = None

    # 2. Check Twitter-specific port file
    port = _read_port_file()
    if port and _port_alive(port):
        log.info("[twitter] Reusing visible Chrome on port %d", port)
        _visible_tab = _open_tab(port)
        return _visible_tab

    # 3. Check NEOBROWSER_ATTACH_PORT (user may have a real Chrome open)
    import os
    env_port = os.environ.get("NEOBROWSER_ATTACH_PORT")
    if env_port:
        p = int(env_port)
        if _port_alive(p):
            log.info("[twitter] Using NEOBROWSER_ATTACH_PORT=%d", p)
            _write_port_file(p)
            _visible_tab = _open_tab(p)
            return _visible_tab

    # 4. Scan for any running neorender visible Chrome
    for candidate_file in [
        Path.home() / ".neorender" / "neo-browser-port.txt",
    ]:
        try:
            p = int(candidate_file.read_text().strip())
            if _port_alive(p):
                log.info("[twitter] Found existing Chrome on port %d via %s", p, candidate_file)
                _write_port_file(p)
                _visible_tab = _open_tab(p)
                return _visible_tab
        except Exception:
            pass

    # 5. Launch new visible Chrome — user must login to Twitter once
    from neobrowser.chrome_process import ChromeProcess, wait_for_chrome, PROFILES_BASE

    profile_dir = PROFILES_BASE / "twitter-visible"
    chrome = ChromeProcess.launch_visible(profile_dir)
    if not wait_for_chrome(chrome.port, timeout_s=15.0):
        chrome.kill(force=True)
        raise RuntimeError(
            "Visible Chrome did not start within 15s. "
            "Alternatively, set NEOBROWSER_ATTACH_PORT to a Chrome with Twitter session."
        )

    _visible_pid = chrome.pid
    _write_port_file(chrome.port)
    log.info("[twitter] Launched NEW visible Chrome: port=%d pid=%d — user must login to Twitter", chrome.port, chrome.pid)

    _visible_tab = _open_tab(chrome.port)
    return _visible_tab


def _open_tab(port: int) -> "ChromeTab":
    from neobrowser.chrome_tab import ChromeTab
    return ChromeTab.open(port)


def _port_alive(port: int) -> bool:
    try:
        import urllib.request
        urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=2.0)
        return True
    except Exception:
        return False


def _read_port_file() -> int | None:
    try:
        return int(_PORT_FILE.read_text().strip())
    except Exception:
        return None


def _write_port_file(port: int) -> None:
    _PORT_FILE.parent.mkdir(parents=True, exist_ok=True)
    _PORT_FILE.write_text(str(port))


# ---------------------------------------------------------------------------
# Login wall dismissal
# ---------------------------------------------------------------------------

_DISMISS_LOGIN_WALL_JS = """
(function() {
    const btns = Array.from(document.querySelectorAll('[role="button"], button'));
    const close = btns.find(b =>
        /^(close|×|✕|not now|no thanks)/i.test(b.innerText?.trim())
        || b.getAttribute('aria-label')?.match(/close/i)
    );
    if (close) { close.click(); return 'dismissed'; }
    return 'none';
})();
"""

# ---------------------------------------------------------------------------
# Video tweet extraction JS
# ---------------------------------------------------------------------------

_TWITTER_VIDEO_EXTRACT_JS = """
return (function(count) {
    const results = [];
    const seen = new Set();

    const tweets = Array.from(document.querySelectorAll(
        'article[data-testid="tweet"], article[role="article"]'
    ));

    for (const tweet of tweets) {
        if (results.length >= count) break;

        // Tweet URL from timestamp link
        const timeLink = tweet.querySelector('a[href*="/status/"] time')?.closest('a');
        const tweetUrl = timeLink?.href || '';
        if (!tweetUrl || seen.has(tweetUrl)) continue;
        seen.add(tweetUrl);

        // Must have video
        const hasVideo = tweet.querySelector(
            'video, [data-testid="videoPlayer"], [data-testid="videoComponent"]'
        );
        if (!hasVideo) continue;

        // Author
        const authorLink = tweet.querySelector('a[role="link"][href^="/"]');
        const handle = authorLink?.href?.replace(/.*\\.com\\//, '@') || '';

        // Display name
        const nameEl = tweet.querySelector('[data-testid="User-Name"]');
        const nameParts = nameEl?.innerText?.split('\\n') || [];
        const displayName = nameParts[0] || '';

        // Tweet text
        const textEl = tweet.querySelector('[data-testid="tweetText"]');
        const text = (textEl?.innerText || '').slice(0, 300);

        // Timestamp
        const timeEl = tweet.querySelector('time');
        const timestamp = timeEl?.getAttribute('datetime') || timeEl?.innerText || '';

        // Metrics
        const metricEls = tweet.querySelectorAll(
            '[data-testid="reply"], [data-testid="retweet"], [data-testid="like"], [role="group"] span'
        );
        const metricTexts = Array.from(metricEls).map(
            el => el.getAttribute('aria-label') || el.innerText || ''
        );
        const metrics = { replies: '', reposts: '', likes: '', views: '' };
        for (const m of metricTexts) {
            const lower = m.toLowerCase();
            if (lower.includes('repl')) metrics.replies = m.replace(/[^0-9KMkm.]/g, '');
            else if (lower.includes('repost') || lower.includes('retweet'))
                metrics.reposts = m.replace(/[^0-9KMkm.]/g, '');
            else if (lower.includes('like')) metrics.likes = m.replace(/[^0-9KMkm.]/g, '');
            else if (lower.includes('view')) metrics.views = m.replace(/[^0-9KMkm.]/g, '');
        }

        results.push({ author: handle, display_name: displayName, text, url: tweetUrl, timestamp, metrics });
    }

    return results;
})(REPLACE_COUNT);
"""


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

def search_twitter_videos(query: str, count: int = 10) -> list[dict]:
    """
    Search Twitter/X for video tweets using a visible Chrome instance.

    Manages its own browser — does NOT need a tab from the server.
    The visible Chrome persists across calls (port saved to disk).

    Args:
        query:  Search query string.
        count:  Max number of results (default 10, max 30).

    Returns:
        List of dicts with keys:
          author, display_name, text, url, timestamp, metrics, download_cmd
    """
    count = min(count, 30)
    tab = _get_visible_tab()

    # Navigate to Twitter video search
    q = urllib.parse.quote_plus(query)
    url = f"https://x.com/search?q={q}&src=typed_query&f=video"
    tab.navigate(url, wait_s=5.0)

    # Dismiss login wall if present
    try:
        tab.js(_DISMISS_LOGIN_WALL_JS)
    except Exception:
        pass

    # Scroll to load more results
    for _ in range(3):
        try:
            tab.js("window.scrollBy(0, window.innerHeight)")
            time.sleep(1.2)
        except Exception:
            break

    # Extract video tweets
    js = _TWITTER_VIDEO_EXTRACT_JS.replace("REPLACE_COUNT", str(count))
    raw: list[dict] = tab.js(js) or []

    results = []
    for r in raw:
        tweet_url = r.get("url", "")
        results.append({
            "author": r.get("author", ""),
            "display_name": r.get("display_name", ""),
            "text": r.get("text", ""),
            "url": tweet_url,
            "timestamp": r.get("timestamp", ""),
            "metrics": r.get("metrics", {}),
            "download_cmd": _download_cmd(tweet_url),
        })

    return results


def _download_cmd(url: str) -> str:
    """Return a yt-dlp command for a Twitter video URL."""
    if not url or ("x.com" not in url and "twitter.com" not in url):
        return ""
    safe_name = re.sub(r'[^\w-]', '_', url.split("/status/")[-1][:20]) or "tweet_video"
    return f'yt-dlp -o "{safe_name}.%(ext)s" "{url}"'
