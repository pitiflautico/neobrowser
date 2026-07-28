"""Tests for the perception helpers (pure functions)."""
from neobrowser.perception import click_outcome, classify_page_state


def test_click_outcome_navigated():
    o, extra = click_outcome({"url": "a", "modals": 0, "bodyLen": 100},
                             {"url": "b", "modals": 0, "bodyLen": 100})
    assert o == "navigated" and extra["new_url"] == "b"


def test_click_outcome_modal_opened():
    o, _ = click_outcome({"url": "a", "modals": 0, "bodyLen": 100},
                         {"url": "a", "modals": 1, "bodyLen": 100})
    assert o == "modal_opened"


def test_click_outcome_page_updated():
    o, _ = click_outcome({"url": "a", "modals": 0, "bodyLen": 100},
                         {"url": "a", "modals": 0, "bodyLen": 900})
    assert o == "page_updated"


def test_click_outcome_no_change():
    o, _ = click_outcome({"url": "a", "modals": 0, "bodyLen": 100},
                         {"url": "a", "modals": 0, "bodyLen": 120})
    assert o == "no_change"


def test_classify_login():
    assert classify_page_state("Please sign in with your password to continue") == "login_required"


def test_classify_captcha():
    assert classify_page_state("Verify you are human — complete the captcha challenge") == "captcha"


def test_classify_rate_limited():
    assert classify_page_state("Too many requests (429). Please retry later.") == "rate_limited"


def test_classify_normal_page_is_none():
    assert classify_page_state("Welcome to the documentation. Here is how to get started.") is None
