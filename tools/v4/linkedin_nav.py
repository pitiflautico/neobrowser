"""
tools/v4/linkedin_nav.py

Tier 3: LinkedIn SPA navigation helper.

Root cause of V3 watcher failure:
- Direct URL navigation to /messaging/thread/<id> does NOT render the thread
  in headless Chrome. The SPA requires you to arrive at /messaging/ first,
  then click the conversation in the sidebar.

This module encapsulates that sequence so the watcher and any future tool
can reliably open a LinkedIn thread and read/send messages.

V4 changes vs V3:
- Bug 1: sidebar selector fixed to .msg-conversations-container__conversations-list
- Bug 2: thread click uses PageAnalyzer.find_by_intent + JS fallback (no href)
- Bug 3: send_message uses PageAnalyzer.find_input_box / find_send_button + fallback
"""
from __future__ import annotations

import time

from tools.v4.chrome_tab import ChromeTab

# CSS selectors for LinkedIn's messaging UI (as of 2026-Q1).
_SEL_MSG_BODY = ".msg-s-event-listitem__body"
# Bug 1 fix: was ".msg-conversations-container" (does not exist in DOM).
_SEL_SIDEBAR_LOADED = ".msg-conversations-container__conversations-list"
# Fallback selectors used when PageAnalyzer returns None.
_SEL_FIRST_CONV = ".msg-conversation-listitem__link"
_SEL_COMPOSE_BOX = ".msg-form__contenteditable"
_SEL_SEND_BUTTON = ".msg-form__send-button"


def _thread_id_from_url(url: str) -> str:
    """
    Extract the thread ID from a LinkedIn messaging URL.

    Accepts:
      https://www.linkedin.com/messaging/thread/2-abc123==/
      2-abc123==
    Returns the bare thread ID.
    """
    if "/messaging/thread/" in url:
        return url.split("/messaging/thread/")[1].strip("/")
    return url.strip("/")


class LinkedInNavigator:
    """
    SPA-aware navigation for LinkedIn messaging.

    PageAnalyzer is injected via the constructor for testability.
    If not provided, one is created internally.

    Usage:
        nav = LinkedInNavigator()
        opened = nav.open_thread(tab, "2-abc123==")
        if opened:
            last = nav.get_last_message(tab)
    """

    def __init__(self, analyzer=None):
        if analyzer is not None:
            self._analyzer = analyzer
        else:
            from tools.v4.page_analyzer import PageAnalyzer
            self._analyzer = PageAnalyzer()

    # ------------------------------------------------------------------
    # open_thread
    # ------------------------------------------------------------------

    def open_thread(
        self,
        tab: ChromeTab,
        thread_ref: str,
        sidebar_timeout_s: float = 12.0,
        thread_timeout_s: float = 8.0,
    ) -> bool:
        """
        Open a LinkedIn messaging thread reliably.

        Steps:
          1. Navigate to /messaging/ (SPA entry point)
          2. Wait for the conversation sidebar to load
          3. Click the target thread via PageAnalyzer, with JS fallback
          4. Wait for the message list to appear

        Parameters
        ----------
        tab:               An open ChromeTab with a LinkedIn-authenticated session.
        thread_ref:        Full thread URL or bare thread ID (e.g. "2-abc123==").
        sidebar_timeout_s: Seconds to wait for the sidebar after navigation.
        thread_timeout_s:  Seconds to wait for messages after clicking sidebar.

        Returns True if the thread was opened and messages are visible.
        """
        thread_id = _thread_id_from_url(thread_ref)

        # Step 1: navigate to the messaging hub (SPA entry point).
        tab.navigate("https://www.linkedin.com/messaging/", wait_s=2.0)

        # Step 2: wait for sidebar to render (Bug 1 fix: correct selector).
        if not tab.wait_for_selector(_SEL_SIDEBAR_LOADED, timeout_s=sidebar_timeout_s):
            return False

        # Step 3: click the target conversation.
        # Bug 2 fix: LinkedIn sidebar uses div.msg-conversation-listitem__link,
        # no href attribute and no thread ID in DOM — cannot use a[href*=...].
        # Use PageAnalyzer to find by intent; fall back to clicking the first link.
        # Sanitize thread_id before interpolating into LLM prompt (prompt injection defence).
        # LinkedIn thread IDs are base64url — strip anything outside that alphabet.
        import re as _re
        safe_thread_id = _re.sub(r"[^A-Za-z0-9+/=_\-]", "", thread_id)[:128]
        intent = f"click on conversation for thread {safe_thread_id} in sidebar"
        node_id = self._analyzer.find_by_intent(tab, intent)
        if node_id is not None:
            self._analyzer.click_node(tab, node_id)
        else:
            # Fallback: click the first visible conversation link.
            tab.js(
                "var el=document.querySelector('.msg-conversation-listitem__link');"
                " if(el) el.click();"
            )

        # Step 4: wait for message list to appear.
        return tab.wait_for_selector(_SEL_MSG_BODY, timeout_s=thread_timeout_s)

    # ------------------------------------------------------------------
    # get_last_message
    # ------------------------------------------------------------------

    def get_last_message(
        self,
        tab: ChromeTab,
        timeout_s: float = 5.0,
    ) -> str | None:
        """
        Return the text of the last visible message in the open thread.

        Tries PageAnalyzer.find_last_message() (AX tree — more reliable) first.
        Falls back to tab.wait_last() on the CSS selector.

        Returns None if no message is found within timeout_s.
        """
        result = self._analyzer.find_last_message(tab)
        if result:
            return result
        return tab.wait_last(_SEL_MSG_BODY, timeout_s=timeout_s)

    # ------------------------------------------------------------------
    # send_message
    # ------------------------------------------------------------------

    def send_message(
        self,
        tab: ChromeTab,
        text: str,
        send_timeout_s: float = 5.0,
    ) -> bool:
        """
        Type and send a message in the currently open thread.

        Steps:
          1. Find input box via PageAnalyzer; fallback to CSS wait + click
          2. Type the message
          3. Find send button via PageAnalyzer; fallback to CSS click
          4. Click send

        Returns True if the send button was clicked successfully.
        """
        # Step 1 & 2: locate input box and type.
        input_node = self._analyzer.find_input_box(tab)
        if input_node is not None:
            self._analyzer.type_in_node(tab, input_node, text)
        else:
            # CSS fallback.
            if not tab.wait_for_selector(_SEL_COMPOSE_BOX, timeout_s=send_timeout_s):
                return False
            tab.click(_SEL_COMPOSE_BOX)
            time.sleep(0.3)
            tab.send("Input.insertText", {"text": text})

        time.sleep(0.2)

        # Step 3 & 4: locate send button and click.
        send_node = self._analyzer.find_send_button(tab)
        if send_node is not None:
            return self._analyzer.click_node(tab, send_node)
        else:
            return tab.click(_SEL_SEND_BUTTON)
