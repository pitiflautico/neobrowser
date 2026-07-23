"""
tools/v4/google_search.py

Google Images and Videos search for NeoBrowser V4.

Two public functions designed to be called from the MCP server tools:

    search_images(tab, query, count) → list[ImageResult]
    search_videos(tab, query, count) → list[VideoResult]

Each result is a plain dict AI-agents can reason over directly:

ImageResult
-----------
  title        str   — alt text / filename hint
  image_url    str   — direct URL to the original image (downloadable)
  source_url   str   — web page where the image lives
  source_host  str   — domain name (e.g. "unsplash.com")
  description  str   — surrounding context text when available
  download_cmd str   — shell command: `curl -L -o <filename> <url>`

VideoResult
-----------
  title        str   — video title
  url          str   — watch URL (YouTube / Vimeo / Instagram / TikTok …)
  channel      str   — uploader / channel name
  duration     str   — "MM:SS" or "HH:MM:SS"
  description  str   — snippet shown by Google
  platform     str   — "youtube" | "instagram" | "tiktok" | "vimeo" | "other"
  download_cmd str   — yt-dlp command for supported platforms; empty otherwise

Google search parameters used
------------------------------
  Images: https://www.google.com/search?q=<query>&udm=2  (Google Lens / Images tab)
  Videos: https://www.google.com/search?q=<query>&udm=7  (Google Videos tab)

Notes
-----
- Google may show a consent dialog on first visit; this module auto-dismisses it.
- Image URLs are extracted from Google's embedded script data (not thumbnails).
- Deduplication is applied: same image_url or video url appears only once.
- yt-dlp must be installed for download_cmd to work: `pip install yt-dlp`
"""
from __future__ import annotations

import re
import urllib.parse
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from neobrowser.chrome_tab import ChromeTab


# ---------------------------------------------------------------------------
# Consent dialog
# ---------------------------------------------------------------------------

_DISMISS_CONSENT_JS = """
(function() {
    const btns = Array.from(document.querySelectorAll('button, [role="button"]'));
    const accept = btns.find(b =>
        /accept all|aceptar todo|tout accepter|alle akzeptieren/i.test(b.innerText)
    );
    if (accept) { accept.click(); return true; }
    return false;
})();
"""


def _dismiss_consent(tab: "ChromeTab") -> None:
    try:
        tab.js(_DISMISS_CONSENT_JS)
    except Exception:
        pass


# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------

def _google_url(query: str, udm: int) -> str:
    q = urllib.parse.quote_plus(query)
    return f"https://www.google.com/search?q={q}&udm={udm}&num=30"


def _download_cmd_for_video(url: str, title: str) -> str:
    """Return a yt-dlp shell command for the given video URL, or '' if unsupported."""
    supported = ("youtube.com", "youtu.be", "vimeo.com", "instagram.com",
                 "tiktok.com", "twitter.com", "x.com", "facebook.com", "dailymotion.com")
    if not any(p in url for p in supported):
        return ""
    # Sanitize title for filename
    safe = re.sub(r'[^\w\s-]', '', title).strip().replace(' ', '_')[:60] or "video"
    return f'yt-dlp -o "{safe}.%(ext)s" "{url}"'


def _download_cmd_for_image(url: str) -> str:
    """Return a curl command to download the image."""
    if not url or not url.startswith("http"):
        return ""
    ext = re.search(r'\.(jpg|jpeg|png|webp|gif)(?:\?|$)', url, re.I)
    ext = ext.group(1).lower() if ext else "jpg"
    name = re.sub(r'[^\w-]', '_', urllib.parse.urlparse(url).path.split('/')[-1])[:40] or f"image.{ext}"
    return f'curl -L -o "{name}" "{url}"'


def _platform(url: str) -> str:
    mapping = {
        "youtube.com": "youtube", "youtu.be": "youtube",
        "instagram.com": "instagram", "tiktok.com": "tiktok",
        "vimeo.com": "vimeo", "twitter.com": "twitter", "x.com": "twitter",
        "facebook.com": "facebook", "dailymotion.com": "dailymotion",
    }
    for host, name in mapping.items():
        if host in url:
            return name
    return "other"


# ---------------------------------------------------------------------------
# Image search
# ---------------------------------------------------------------------------

_IMAGE_EXTRACT_JS = """
return (function(count) {
    const results = [];
    const seen = new Set();

    // ── Strategy 1: extract original image URLs from embedded script data ──
    // Google embeds image metadata as URL strings inside <script> blocks.
    const scriptText = Array.from(document.querySelectorAll('script'))
        .map(s => s.text).join('\\n');

    const imgPattern = /https?:\\/\\/(?!encrypted-tbn)[\\w.\\-/%?=&+@#!:,;~]+\\.(?:jpg|jpeg|png|webp)(?:[?&][^"'\\s<>\\\\]{0,120})?/gi;
    const rawUrls = [...new Set(scriptText.match(imgPattern) || [])];

    // ── Strategy 2: extract source page links from anchor tags ──
    const sourceMap = {};  // image domain → source page URL
    Array.from(document.querySelectorAll('a[href^="http"]'))
        .filter(a => !a.href.includes('google.com') && !a.href.includes('gstatic.com'))
        .forEach(a => {
            const text = a.innerText?.trim();
            if (text && a.href) sourceMap[a.href] = text;
        });

    const sourcePairs = Array.from(document.querySelectorAll('a[href^="http"]'))
        .filter(a => !a.href.includes('google.com'))
        .map(a => ({href: a.href, text: a.innerText?.trim() || ''}));

    // ── Strategy 3: get title/alt from visible image elements ──
    const imgMeta = {};
    Array.from(document.querySelectorAll('img[alt], img[title]')).forEach(img => {
        const key = (img.src || '').split('?')[0];
        if (key) imgMeta[key] = img.alt || img.title || '';
    });

    // Filter and deduplicate
    const filtered = rawUrls
        .filter(u => !u.includes('gstatic.com') && !u.includes('google.com')
                  && !u.includes('googleapis.com') && u.length > 30)
        .slice(0, count * 3);

    for (const imgUrl of filtered) {
        if (seen.has(imgUrl)) continue;
        seen.add(imgUrl);

        const urlObj = new URL(imgUrl);
        const host = urlObj.hostname.replace(/^www\\./, '');

        // Find best source page for this image (same hostname)
        const sourcePair = sourcePairs.find(p => {
            try { return new URL(p.href).hostname.includes(host) || host.includes(new URL(p.href).hostname.replace(/^www\\./, '')); }
            catch { return false; }
        });

        results.push({
            image_url: imgUrl,
            source_url: sourcePair?.href || '',
            source_host: host,
            title: imgMeta[imgUrl.split('?')[0]] || sourcePair?.text?.split('\\n')[0] || '',
            description: sourcePair?.text || '',
        });

        if (results.length >= count) break;
    }

    return results;
})(REPLACE_COUNT);
"""

_VIDEO_EXTRACT_JS = """
return (function(count) {
    const results = [];
    const seen = new Set();
    const durationRe = /^\\d{1,2}:\\d{2}(:\\d{2})?$/;

    const headings = Array.from(document.querySelectorAll('h3'));

    for (const h3 of headings) {
        if (results.length >= count) break;

        const a = h3.closest('a') || h3.parentElement?.querySelector('a');
        const url = a?.href || '';
        if (!url || seen.has(url)) continue;
        seen.add(url);

        // Walk up to find the card container (has meaningful text)
        let card = h3.parentElement;
        for (let i = 0; i < 8; i++) {
            if (!card) break;
            if (card.innerText?.length > 60) break;
            card = card.parentElement;
        }

        // Parse card innerText line by line — Google Videos structure:
        //   Line 0: Title
        //   Line 1: "www.youtube.com › watch" (URL hint, skip)
        //   Line 2: Duration (MM:SS) — may be absent for Shorts/Reels
        //   Line 3: Description snippet
        //   Line N: "Platform · Channel · Date"
        const lines = (card?.innerText || '').split('\\n')
            .map(l => l.trim()).filter(l => l);

        let duration = '';
        let description = '';
        let channel = '';

        // Skip title and URL hint lines
        const bodyLines = lines.filter(l =>
            l !== h3.innerText && !l.includes('www.') && !l.startsWith('›')
        );

        for (const line of bodyLines) {
            if (!duration && durationRe.test(line)) {
                duration = line;
                continue;
            }
            // "Platform · Channel · Date" pattern
            if (line.includes('·')) {
                const parts = line.split('·').map(p => p.trim());
                // Channel is the second part if short enough to be a name
                if (parts.length >= 2 && !channel) {
                    const candidate = parts[1] || parts[0] || '';
                    // Reject if it looks like metadata ("1 year ago", "1.4M views", etc.)
                    if (candidate.length < 60 && !/\d+\s*(year|month|day|view|ago)/i.test(candidate)) {
                        channel = candidate;
                    }
                }
                continue;
            }
            // Longer lines → description
            if (!description && line.length > 20) {
                description = line;
            }
        }

        results.push({
            title: h3.innerText,
            url,
            channel,
            duration,
            description: description.slice(0, 300),
        });
    }

    return results;
})(REPLACE_COUNT);
"""


def search_images(tab: "ChromeTab", query: str, count: int = 10) -> list[dict]:
    """
    Search Google Images and return structured results.

    Args:
        tab:    Active ChromeTab instance.
        query:  Search query string.
        count:  Max number of results (default 10, max 30).

    Returns:
        List of dicts with keys:
          title, image_url, source_url, source_host, description, download_cmd
    """
    count = min(count, 30)
    url = _google_url(query, udm=2)
    tab.navigate(url, wait_s=3.0)
    _dismiss_consent(tab)

    js = _IMAGE_EXTRACT_JS.replace("REPLACE_COUNT", str(count))
    raw: list[dict] = tab.js(js) or []

    results = []
    for r in raw:
        img_url = r.get("image_url", "")
        results.append({
            "title": r.get("title", ""),
            "image_url": img_url,
            "source_url": r.get("source_url", ""),
            "source_host": r.get("source_host", ""),
            "description": r.get("description", ""),
            "download_cmd": _download_cmd_for_image(img_url),
        })

    return results


def search_videos(tab: "ChromeTab", query: str, count: int = 10) -> list[dict]:
    """
    Search Google Videos and return structured results.

    Args:
        tab:    Active ChromeTab instance.
        query:  Search query string.
        count:  Max number of results (default 10, max 30).

    Returns:
        List of dicts with keys:
          title, url, channel, duration, description, platform, download_cmd
    """
    count = min(count, 30)
    url = _google_url(query, udm=7)
    tab.navigate(url, wait_s=3.0)
    _dismiss_consent(tab)

    js = _VIDEO_EXTRACT_JS.replace("REPLACE_COUNT", str(count))
    raw: list[dict] = tab.js(js) or []

    results = []
    for r in raw:
        video_url = r.get("url", "")
        title = r.get("title", "")
        results.append({
            "title": title,
            "url": video_url,
            "channel": r.get("channel", ""),
            "duration": r.get("duration", ""),
            "description": r.get("description", ""),
            "platform": _platform(video_url),
            "download_cmd": _download_cmd_for_video(video_url, title),
        })

    return results
