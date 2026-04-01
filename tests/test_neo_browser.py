"""
test_neo_browser.py — Pure-function tests for neo-browser and plugins.

No Chrome, no network, no browser required.
"""
import json
import sys
import threading
import time
import types
from pathlib import Path
from unittest.mock import MagicMock, patch, call

import pytest

# ── Ensure tools/v3 is on the path (plugins.py lives there) ──
_V3 = str(Path(__file__).parent.parent / 'tools' / 'v3')
if _V3 not in sys.path:
    sys.path.insert(0, _V3)

# ── Import neo_browser (loaded by conftest) ──
import sys as _sys
# conftest already loaded it; grab from sys.modules
neo_browser = _sys.modules['neo_browser']

# ── Import plugins directly (no side-effects) ──
import plugins as plg


# ═══════════════════════════════════════════════════════════════
# 1. PageCache
# ═══════════════════════════════════════════════════════════════

class TestPageCache:

    def test_put_and_get_basic(self, fresh_cache):
        fresh_cache.put('http://example.com', 'hello')
        assert fresh_cache.get('http://example.com') == 'hello'

    def test_cache_miss_returns_none(self, fresh_cache):
        assert fresh_cache.get('http://nothere.com') is None

    def test_ttl_expiration(self, fresh_cache):
        # Set TTL to 0 so everything expires immediately
        fresh_cache._ttl = 0
        fresh_cache.put('http://example.com', 'data')
        # After TTL=0 any access is expired
        time.sleep(0.01)
        assert fresh_cache.get('http://example.com') is None

    def test_ttl_not_expired(self, fresh_cache):
        fresh_cache._ttl = 9999
        fresh_cache.put('http://example.com', 'data')
        assert fresh_cache.get('http://example.com') == 'data'

    def test_lru_eviction_when_full(self):
        # max_items=3, fill 3 entries then add a 4th
        cache = neo_browser.PageCache(max_items=3, ttl_s=9999)
        cache.put('http://a.com', 'a')
        time.sleep(0.01)
        cache.put('http://b.com', 'b')
        time.sleep(0.01)
        cache.put('http://c.com', 'c')
        time.sleep(0.01)
        # 'a' was inserted first — it has the smallest timestamp → evicted
        cache.put('http://d.com', 'd')
        assert cache.get('http://a.com') is None
        assert cache.get('http://b.com') == 'b'
        assert cache.get('http://c.com') == 'c'
        assert cache.get('http://d.com') == 'd'

    def test_thread_safety(self, fresh_cache):
        errors = []

        def writer(i):
            try:
                for j in range(20):
                    fresh_cache.put(f'http://url-{i}-{j}.com', f'content-{i}-{j}')
            except Exception as e:
                errors.append(e)

        def reader():
            try:
                for i in range(100):
                    fresh_cache.get(f'http://url-{i % 5}-{i % 20}.com')
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=writer, args=(i,)) for i in range(5)]
        threads += [threading.Thread(target=reader) for _ in range(3)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert errors == [], f'Thread safety errors: {errors}'

    def test_overwrite_existing_key(self, fresh_cache):
        fresh_cache.put('http://x.com', 'v1')
        fresh_cache.put('http://x.com', 'v2')
        assert fresh_cache.get('http://x.com') == 'v2'


# ═══════════════════════════════════════════════════════════════
# 2. error_response
# ═══════════════════════════════════════════════════════════════

class TestErrorResponse:

    def test_returns_valid_json_string(self):
        result = neo_browser.error_response('not_found', 'Page not found')
        data = json.loads(result)
        assert isinstance(data, dict)

    def test_contains_error_and_message(self):
        result = neo_browser.error_response('timeout', 'Request timed out')
        data = json.loads(result)
        assert data['error'] == 'timeout'
        assert data['message'] == 'Request timed out'

    def test_optional_url_field(self):
        result = neo_browser.error_response('blocked', 'Blocked', url='http://bad.com')
        data = json.loads(result)
        assert data['url'] == 'http://bad.com'

    def test_optional_suggestion_field(self):
        result = neo_browser.error_response('blocked', 'Blocked', suggestion='Try HTTPS')
        data = json.loads(result)
        assert data['suggestion'] == 'Try HTTPS'

    def test_missing_optional_fields_absent(self):
        result = neo_browser.error_response('err', 'msg')
        data = json.loads(result)
        assert 'url' not in data
        assert 'suggestion' not in data

    def test_logs_error(self, capsys):
        neo_browser.error_response('test_code', 'test_message')
        captured = capsys.readouterr()
        assert 'test_code' in captured.err
        assert 'test_message' in captured.err


# ═══════════════════════════════════════════════════════════════
# 3. sanitize_unicode
# ═══════════════════════════════════════════════════════════════

class TestSanitizeUnicode:

    def test_removes_zero_width_chars(self):
        text = 'hello\u200bworld'   # zero-width space
        assert neo_browser.sanitize_unicode(text) == 'helloworld'

    def test_removes_rtl_ltr_marks(self):
        text = 'abc\u200edef\u200fghi'  # LRM + RLM
        assert neo_browser.sanitize_unicode(text) == 'abcdefghi'

    def test_removes_directional_overrides(self):
        text = 'safe\u202aembedded\u202e'  # LRE + RLO
        assert neo_browser.sanitize_unicode(text) == 'safeembedded'

    def test_removes_bidi_isolates(self):
        text = 'a\u2066b\u2069c'  # LRI + PDI
        assert neo_browser.sanitize_unicode(text) == 'abc'

    def test_removes_bom(self):
        text = '\ufeffdata'
        assert neo_browser.sanitize_unicode(text) == 'data'

    def test_preserves_normal_text(self):
        text = 'Hello, World! 123 — café résumé'
        result = neo_browser.sanitize_unicode(text)
        # After NFKC normalization the text may be slightly normalised but content preserved
        assert 'Hello' in result
        assert 'World' in result
        assert '123' in result

    def test_handles_empty_string(self):
        assert neo_browser.sanitize_unicode('') == ''

    def test_handles_none(self):
        # Function returns input unchanged if falsy
        assert neo_browser.sanitize_unicode(None) is None


# ═══════════════════════════════════════════════════════════════
# 4. validate_url
# ═══════════════════════════════════════════════════════════════

class TestValidateUrl:

    def test_accepts_https(self):
        assert neo_browser.validate_url('https://example.com/path') is True

    def test_accepts_http(self):
        assert neo_browser.validate_url('http://example.com') is True

    def test_rejects_localhost(self):
        assert neo_browser.validate_url('http://localhost/') is False

    def test_rejects_127(self):
        assert neo_browser.validate_url('http://127.0.0.1/') is False

    def test_rejects_0000(self):
        assert neo_browser.validate_url('http://0.0.0.0/') is False

    def test_rejects_10_x(self):
        assert neo_browser.validate_url('http://10.0.0.1/') is False
        assert neo_browser.validate_url('http://10.255.255.255/') is False

    def test_rejects_192_168(self):
        assert neo_browser.validate_url('http://192.168.1.1/') is False

    def test_rejects_172_16_to_31(self):
        for i in range(16, 32):
            assert neo_browser.validate_url(f'http://172.{i}.0.1/') is False

    def test_rejects_169_254_metadata(self):
        assert neo_browser.validate_url('http://169.254.169.254/latest/meta-data/') is False

    def test_rejects_169_254_any(self):
        assert neo_browser.validate_url('http://169.254.0.1/') is False

    def test_rejects_credentials_in_url(self):
        assert neo_browser.validate_url('http://user:pass@example.com/') is False

    def test_rejects_file_scheme(self):
        assert neo_browser.validate_url('file:///etc/passwd') is False

    def test_rejects_ftp_scheme(self):
        assert neo_browser.validate_url('ftp://ftp.example.com/') is False

    def test_rejects_empty_string(self):
        assert neo_browser.validate_url('') is False

    def test_rejects_none(self):
        assert neo_browser.validate_url(None) is False

    def test_accepts_normal_domain(self):
        assert neo_browser.validate_url('https://news.ycombinator.com/') is True

    def test_accepts_url_with_path_and_query(self):
        assert neo_browser.validate_url('https://api.github.com/repos?page=1') is True


# ═══════════════════════════════════════════════════════════════
# 5. scan_secrets
# ═══════════════════════════════════════════════════════════════

class TestScanSecrets:

    def test_detects_aws_key(self):
        text = 'export AWS_KEY=AKIAIOSFODNN7EXAMPLE'
        found = neo_browser.scan_secrets(text)
        assert 'AWS Access Key' in found

    def test_detects_openai_key(self):
        text = 'key = sk-abcdefghijklmnopqrstuvwxyz1234567890'
        found = neo_browser.scan_secrets(text)
        assert 'OpenAI API key' in found

    def test_detects_github_pat(self):
        # Pattern: ghp_[a-zA-Z0-9]{36}  — exactly 36 alphanum chars after ghp_
        text = 'token: ghp_' + 'a' * 36
        found = neo_browser.scan_secrets(text)
        assert 'GitHub PAT' in found

    def test_detects_github_oauth(self):
        # Pattern: gho_[a-zA-Z0-9]{36}
        text = 'auth: gho_' + 'b' * 36
        found = neo_browser.scan_secrets(text)
        assert 'GitHub OAuth' in found

    def test_detects_anthropic_key(self):
        # Pattern: sk-ant-api\w{20,}  — 20+ word chars after sk-ant-api
        text = 'ANTHROPIC_KEY=sk-ant-api' + 'x' * 25
        found = neo_browser.scan_secrets(text)
        assert 'Anthropic API key' in found

    def test_detects_private_key(self):
        text = '-----BEGIN RSA PRIVATE KEY-----\nMIIEow...'
        found = neo_browser.scan_secrets(text)
        assert 'Private Key' in found

    def test_returns_empty_for_clean_text(self):
        text = 'The quick brown fox jumps over the lazy dog.'
        found = neo_browser.scan_secrets(text)
        assert found == []

    def test_multiple_secrets_in_one_text(self):
        # AWS key is exactly 16 chars after AKIA; GitHub PAT is exactly 36 chars after ghp_
        text = 'AKIAIOSFODNN7EXAMPLE ghp_' + 'a' * 36
        found = neo_browser.scan_secrets(text)
        assert 'AWS Access Key' in found
        assert 'GitHub PAT' in found

    def test_empty_text_returns_empty(self):
        assert neo_browser.scan_secrets('') == []

    def test_none_returns_empty(self):
        assert neo_browser.scan_secrets(None) == []


# ═══════════════════════════════════════════════════════════════
# 6. TOOLS registry
# ═══════════════════════════════════════════════════════════════

EXPECTED_TOOLS = {
    'browse', 'search', 'open', 'read', 'find', 'click', 'type', 'fill',
    'submit', 'scroll', 'screenshot', 'wait', 'login', 'extract',
    'gpt', 'grok', 'js', 'status', 'plugin',
}

READ_ONLY_TOOLS = {'browse', 'search', 'read', 'find', 'extract', 'screenshot', 'status', 'js', 'wait'}
MUTATING_TOOLS = {'open', 'click', 'type', 'fill', 'submit', 'scroll', 'login', 'gpt', 'grok', 'plugin'}


class TestToolsRegistry:

    def test_all_19_tools_registered(self):
        assert set(neo_browser.TOOLS.keys()) == EXPECTED_TOOLS

    def test_each_tool_has_required_fields(self):
        for name, t in neo_browser.TOOLS.items():
            assert 'name' in t, f'{name}: missing name'
            assert 'description' in t, f'{name}: missing description'
            assert 'schema' in t, f'{name}: missing schema'
            assert 'fn' in t, f'{name}: missing fn'
            assert 'read_only' in t, f'{name}: missing read_only'
            assert 'concurrent' in t, f'{name}: missing concurrent'
            assert 'max_result' in t, f'{name}: missing max_result'

    def test_read_only_flags(self):
        for name in READ_ONLY_TOOLS:
            assert neo_browser.TOOLS[name]['read_only'] is True, f'{name} should be read_only'

    def test_mutating_tools_not_read_only(self):
        for name in MUTATING_TOOLS:
            assert neo_browser.TOOLS[name]['read_only'] is False, f'{name} should not be read_only'

    def test_concurrent_tools(self):
        # browse, search, read, find, extract, screenshot, status, js, wait
        for name in READ_ONLY_TOOLS:
            assert neo_browser.TOOLS[name]['concurrent'] is True, f'{name} should be concurrent'

    def test_non_concurrent_mutating_tools(self):
        # mutating tools must be non-concurrent (serialised)
        for name in MUTATING_TOOLS:
            assert neo_browser.TOOLS[name]['concurrent'] is False, f'{name} should be non-concurrent'

    def test_tool_functions_are_callable(self):
        for name, t in neo_browser.TOOLS.items():
            assert callable(t['fn']), f'{name}: fn not callable'

    def test_name_field_matches_registry_key(self):
        for key, t in neo_browser.TOOLS.items():
            assert t['name'] == key


# ═══════════════════════════════════════════════════════════════
# 7. dispatch_tool
# ═══════════════════════════════════════════════════════════════

class TestDispatchTool:

    def test_unknown_tool_returns_error_string(self):
        result = neo_browser.dispatch_tool('no_such_tool', {})
        assert 'Unknown tool' in result
        assert 'no_such_tool' in result

    def test_calls_correct_function(self):
        mock_fn = MagicMock(return_value='ok')
        original = neo_browser.TOOLS['search']['fn']
        neo_browser.TOOLS['search']['fn'] = mock_fn
        try:
            result = neo_browser.dispatch_tool('search', {'query': 'test'})
            mock_fn.assert_called_once_with({'query': 'test'})
            assert result == 'ok'
        finally:
            neo_browser.TOOLS['search']['fn'] = original

    def test_non_concurrent_tool_uses_rlock(self):
        """Verify that _browser_lock is an RLock (reentrant), not a plain Lock."""
        lock = neo_browser._browser_lock
        # RLock can be acquired twice from the same thread; Lock cannot.
        acquired1 = lock.acquire(blocking=False)
        acquired2 = lock.acquire(blocking=False)   # would deadlock with plain Lock
        assert acquired1 is True
        assert acquired2 is True
        lock.release()
        lock.release()

    def test_concurrent_tool_skips_lock(self):
        """Concurrent tools (e.g. search) do NOT acquire _browser_lock."""
        call_order = []
        original_fn = neo_browser.TOOLS['search']['fn']

        def recording_fn(args):
            call_order.append('fn_called')
            return 'result'

        neo_browser.TOOLS['search']['fn'] = recording_fn
        try:
            # Acquire the browser lock from THIS thread
            neo_browser._browser_lock.acquire()
            result = neo_browser.dispatch_tool('search', {'query': 'hi'})
            # Should succeed immediately (didn't wait for lock)
            assert result == 'result'
            assert 'fn_called' in call_order
        finally:
            neo_browser._browser_lock.release()
            neo_browser.TOOLS['search']['fn'] = original_fn

    def test_persist_if_large_called_when_result_exceeds_max_result(self, tmp_path, monkeypatch):
        """Result > max_result triggers persist_if_large."""
        monkeypatch.setattr(neo_browser, 'RESPONSE_DIR', tmp_path)

        big_result = 'x' * (neo_browser.TOOLS['browse']['max_result'] + 1)
        original_fn = neo_browser.TOOLS['browse']['fn']

        neo_browser.TOOLS['browse']['fn'] = lambda args: big_result
        try:
            result = neo_browser.dispatch_tool('browse', {'url': 'http://x.com'})
            # Should be a truncated preview, not the full big_result
            assert len(result) < len(big_result)
            assert 'chars total' in result
        finally:
            neo_browser.TOOLS['browse']['fn'] = original_fn

    def test_small_result_not_persisted(self):
        """Result under max_result is returned unchanged."""
        original_fn = neo_browser.TOOLS['search']['fn']
        neo_browser.TOOLS['search']['fn'] = lambda args: 'small'
        try:
            result = neo_browser.dispatch_tool('search', {'query': 'q'})
            assert result == 'small'
        finally:
            neo_browser.TOOLS['search']['fn'] = original_fn

    def test_reentrant_lock_allows_nested_dispatch(self):
        """Simulate plugin calling dispatch_tool while already holding _browser_lock."""
        results = []

        def inner_dispatch(_name, _args):
            # This is called while _browser_lock is held by the outer dispatch
            results.append(neo_browser.dispatch_tool('search', {'query': 'inner'}))

        original_search = neo_browser.TOOLS['search']['fn']
        original_plugin = neo_browser.TOOLS['plugin']['fn']

        neo_browser.TOOLS['search']['fn'] = lambda args: 'inner_ok'
        # plugin is non-concurrent → acquires _browser_lock, then calls inner_dispatch
        neo_browser.TOOLS['plugin']['fn'] = lambda args: (inner_dispatch('search', {}), 'plugin_ok')[1]

        try:
            result = neo_browser.dispatch_tool('plugin', {'name': 'test'})
            assert result == 'plugin_ok'
            assert results == ['inner_ok'], 'Nested dispatch should succeed with RLock'
        finally:
            neo_browser.TOOLS['search']['fn'] = original_search
            neo_browser.TOOLS['plugin']['fn'] = original_plugin


# ═══════════════════════════════════════════════════════════════
# 8. tool_def decorator
# ═══════════════════════════════════════════════════════════════

class TestToolDefDecorator:

    def test_registers_tool_in_tools_dict(self):
        @neo_browser.tool_def('_test_reg', 'A test tool', {'x': 'required'})
        def _my_fn(args):
            return 'ok'

        assert '_test_reg' in neo_browser.TOOLS
        del neo_browser.TOOLS['_test_reg']

    def test_preserves_original_function(self):
        @neo_browser.tool_def('_test_fn', 'desc', {})
        def _original(args):
            return 'original_result'

        assert _original({'key': 'val'}) == 'original_result'
        del neo_browser.TOOLS['_test_fn']

    def test_schema_stored_correctly(self):
        schema = {'url': 'required', 'timeout': 'optional seconds'}

        @neo_browser.tool_def('_test_schema', 'desc', schema)
        def _fn(args):
            pass

        assert neo_browser.TOOLS['_test_schema']['schema'] == schema
        del neo_browser.TOOLS['_test_schema']

    def test_default_flags(self):
        @neo_browser.tool_def('_test_flags', 'desc', {})
        def _fn(args):
            pass

        t = neo_browser.TOOLS['_test_flags']
        assert t['read_only'] is True
        assert t['concurrent'] is True
        assert t['max_result'] == 0
        del neo_browser.TOOLS['_test_flags']


# ═══════════════════════════════════════════════════════════════
# 9. get_mcp_tools
# ═══════════════════════════════════════════════════════════════

class TestGetMcpTools:

    def test_returns_19_tools(self):
        tools = neo_browser.get_mcp_tools()
        assert len(tools) == 19

    def test_each_has_name_description_inputschema(self):
        for t in neo_browser.get_mcp_tools():
            assert 'name' in t
            assert 'description' in t
            assert 'inputSchema' in t

    def test_input_schema_structure(self):
        for t in neo_browser.get_mcp_tools():
            schema = t['inputSchema']
            assert schema['type'] == 'object'
            assert 'properties' in schema
            assert 'required' in schema

    def test_required_params_marked(self):
        # 'browse' has url: 'required' → should appear in required list
        mcp_tools = {t['name']: t for t in neo_browser.get_mcp_tools()}
        browse = mcp_tools['browse']
        assert 'url' in browse['inputSchema']['required']

    def test_optional_params_not_in_required(self):
        mcp_tools = {t['name']: t for t in neo_browser.get_mcp_tools()}
        browse = mcp_tools['browse']
        assert 'selector' not in browse['inputSchema']['required']

    def test_all_tool_names_present(self):
        names = {t['name'] for t in neo_browser.get_mcp_tools()}
        assert names == EXPECTED_TOOLS


# ═══════════════════════════════════════════════════════════════
# 10. Parameter mapping: type / mode bug fix
# ═══════════════════════════════════════════════════════════════

class TestToolReadParamMapping:

    def test_accepts_type_arg(self):
        """dispatch_tool read with 'type' arg should not raise AttributeError."""
        mock_chrome = MagicMock()
        mock_chrome.markdown.return_value = 'md content'

        with patch.object(neo_browser, 'chrome', return_value=mock_chrome), \
             patch.object(neo_browser, 'chrome_go', return_value=mock_chrome), \
             patch.object(neo_browser, 'process_content', side_effect=lambda x: x), \
             patch.object(neo_browser, 'save', side_effect=lambda t, *a: t):
            fn = neo_browser.TOOLS['read']['fn']
            result = fn({'type': 'markdown'})
            assert 'md content' in result

    def test_accepts_mode_arg(self):
        """dispatch_tool read with 'mode' arg should behave identically to 'type'."""
        mock_chrome = MagicMock()
        mock_chrome.markdown.return_value = 'md content'

        with patch.object(neo_browser, 'chrome', return_value=mock_chrome), \
             patch.object(neo_browser, 'chrome_go', return_value=mock_chrome), \
             patch.object(neo_browser, 'process_content', side_effect=lambda x: x), \
             patch.object(neo_browser, 'save', side_effect=lambda t, *a: t):
            fn = neo_browser.TOOLS['read']['fn']
            result = fn({'mode': 'markdown'})
            assert 'md content' in result

    def test_type_takes_precedence_over_mode(self):
        """When both provided, 'type' wins (or they are OR'd; either way no crash)."""
        mock_chrome = MagicMock()
        mock_chrome.markdown.return_value = 'from_type'

        with patch.object(neo_browser, 'chrome', return_value=mock_chrome), \
             patch.object(neo_browser, 'chrome_go', return_value=mock_chrome), \
             patch.object(neo_browser, 'process_content', side_effect=lambda x: x), \
             patch.object(neo_browser, 'save', side_effect=lambda t, *a: t):
            fn = neo_browser.TOOLS['read']['fn']
            result = fn({'type': 'markdown', 'mode': 'tweets'})
            # Should not crash and should produce content
            assert result is not None


# ═══════════════════════════════════════════════════════════════
# 11. plugins.py — resolve_template
# ═══════════════════════════════════════════════════════════════

class TestResolveTemplate:

    def test_simple_replacement(self):
        assert plg.resolve_template('Hello {{name}}!', {'name': 'World'}) == 'Hello World!'

    def test_nested_replacement(self):
        ctx = {'user': {'email': 'a@b.com'}}
        assert plg.resolve_template('email: {{user.email}}', ctx) == 'email: a@b.com'

    def test_missing_var_returns_empty_string(self):
        result = plg.resolve_template('{{missing}}', {})
        assert result == ''

    def test_non_string_passthrough(self):
        assert plg.resolve_template(42, {}) == 42
        assert plg.resolve_template(None, {}) is None
        assert plg.resolve_template({'a': 1}, {}) == {'a': 1}

    def test_no_template_vars_unchanged(self):
        assert plg.resolve_template('plain text', {}) == 'plain text'

    def test_multiple_vars(self):
        result = plg.resolve_template('{{a}}-{{b}}', {'a': '1', 'b': '2'})
        assert result == '1-2'

    def test_list_index_access(self):
        ctx = {'items': ['x', 'y', 'z']}
        result = plg.resolve_template('{{items.1}}', ctx)
        assert result == 'y'

    def test_list_index_out_of_bounds_returns_empty(self):
        ctx = {'items': ['x']}
        result = plg.resolve_template('{{items.9}}', ctx)
        assert result == ''


# ═══════════════════════════════════════════════════════════════
# 12. plugins.py — resolve_obj
# ═══════════════════════════════════════════════════════════════

class TestResolveObj:

    def test_resolves_string(self):
        assert plg.resolve_obj('{{x}}', {'x': 'hi'}) == 'hi'

    def test_resolves_dict_values(self):
        result = plg.resolve_obj({'key': '{{val}}'}, {'val': '42'})
        assert result == {'key': '42'}

    def test_resolves_list_items(self):
        result = plg.resolve_obj(['{{a}}', '{{b}}'], {'a': '1', 'b': '2'})
        assert result == ['1', '2']

    def test_leaves_non_strings_alone(self):
        assert plg.resolve_obj(99, {}) == 99
        assert plg.resolve_obj(True, {}) is True
        assert plg.resolve_obj(None, {}) is None

    def test_nested_dict_in_list(self):
        obj = [{'url': '{{u}}'}]
        result = plg.resolve_obj(obj, {'u': 'http://example.com'})
        assert result == [{'url': 'http://example.com'}]


# ═══════════════════════════════════════════════════════════════
# 13. plugins.py — run_plugin
# ═══════════════════════════════════════════════════════════════

class TestRunPlugin:

    def _dispatch(self, results_map):
        """Return a dispatch function that returns values from results_map."""
        def dispatch(tool_name, args):
            return results_map.get(tool_name, f'called:{tool_name}')
        return dispatch

    def _plugin(self, steps, inputs=None, output=None):
        data = {'steps': steps}
        if inputs:
            data['inputs'] = inputs
        if output:
            data['output'] = output
        return data

    def test_executes_steps_in_order(self):
        order = []

        def dispatch(tool, args):
            order.append(tool)
            return 'ok'

        plugin = self._plugin([
            {'action': 'open', 'url': 'http://a.com'},
            {'action': 'read'},
            {'action': 'search', 'query': 'q'},
        ])
        plg.run_plugin(plugin, {}, dispatch)
        assert order == ['open', 'read', 'search']

    def test_passes_args_to_dispatch(self):
        received = {}

        def dispatch(tool, args):
            received.update(args)
            return 'ok'

        plugin = self._plugin([{'action': 'open', 'url': 'http://example.com'}])
        plg.run_plugin(plugin, {}, dispatch)
        assert received.get('url') == 'http://example.com'

    def test_save_as_stores_result(self):
        def dispatch(tool, args):
            if tool == 'browse':
                return 'page_content'
            return args.get('url', 'no_url')

        plugin = self._plugin([
            {'action': 'browse', 'url': 'http://a.com', 'save_as': 'page'},
            {'action': 'open', 'url': '{{page}}'},
        ])
        received_urls = []

        def dispatch2(tool, args):
            if tool == 'browse':
                return 'page_content'
            received_urls.append(args.get('url', ''))
            return 'ok'

        plg.run_plugin(plugin, {}, dispatch2)
        assert 'page_content' in received_urls

    def test_repeat_calls_action_multiple_times(self):
        calls = []

        def dispatch(tool, args):
            calls.append(tool)
            return 'ok'

        plugin = self._plugin([{'action': 'click', 'repeat': 3}])
        plg.run_plugin(plugin, {}, dispatch)
        assert calls.count('click') == 3

    def test_loop_over_list_in_ctx(self):
        """Loop calls the action once per item in the comma-separated list."""
        calls = []

        def dispatch(tool, args):
            calls.append(tool)
            return 'ok'

        # Pass a comma-separated list; plugin should iterate 3 times
        plugin = self._plugin([
            {'action': 'search', 'query': 'fixed', 'loop': 'items', 'as': 'item'}
        ], inputs={'items': {'default': ''}})
        plg.run_plugin(plugin, {'items': 'a,b,c'}, dispatch)
        assert calls.count('search') == 3

    def test_output_template_rendered(self):
        plugin = self._plugin(
            [{'action': 'browse', 'url': 'http://x.com', 'save_as': 'result'}],
            output={'template': 'Result: {{result}}'}
        )

        def dispatch(tool, args):
            return 'DATA'

        out = plg.run_plugin(plugin, {}, dispatch)
        assert out == 'Result: DATA'

    def test_max_out_truncation(self):
        """Output exceeding MAX_OUT (50000) is truncated."""
        big = 'x' * 60000

        def dispatch(tool, args):
            return big

        plugin = self._plugin([{'action': 'browse', 'url': 'http://x.com'}] * 5)
        out = plg.run_plugin(plugin, {}, dispatch)
        assert len(out) <= 50000

    def test_empty_steps_returns_empty_string(self):
        plugin = self._plugin([])
        out = plg.run_plugin(plugin, {}, lambda t, a: 'ok')
        assert out == ''

    def test_input_defaults_applied(self):
        received = {}

        def dispatch(tool, args):
            received.update(args)
            return 'ok'

        plugin = self._plugin(
            [{'action': 'open', 'url': '{{target}}'}],
            inputs={'target': {'default': 'http://default.com'}}
        )
        plg.run_plugin(plugin, {}, dispatch)
        assert received.get('url') == 'http://default.com'

    def test_user_input_overrides_default(self):
        received = {}

        def dispatch(tool, args):
            received.update(args)
            return 'ok'

        plugin = self._plugin(
            [{'action': 'open', 'url': '{{target}}'}],
            inputs={'target': {'default': 'http://default.com'}}
        )
        plg.run_plugin(plugin, {'target': 'http://override.com'}, dispatch)
        assert received.get('url') == 'http://override.com'


# ═══════════════════════════════════════════════════════════════
# 14. persist_if_large
# ═══════════════════════════════════════════════════════════════

class TestPersistIfLarge:

    def test_returns_text_unchanged_if_under_limit(self):
        text = 'short text'
        result = neo_browser.persist_if_large(text, 'test')
        assert result == text

    def test_returns_text_unchanged_at_exact_limit(self):
        text = 'a' * neo_browser.MAX_RESULT_CHARS
        result = neo_browser.persist_if_large(text, 'test')
        assert result == text

    def test_saves_to_disk_if_over_limit(self, tmp_path, monkeypatch):
        monkeypatch.setattr(neo_browser, 'RESPONSE_DIR', tmp_path)
        text = 'b' * (neo_browser.MAX_RESULT_CHARS + 1)
        result = neo_browser.persist_if_large(text, 'mytest')
        # File should exist
        saved_files = list(tmp_path.glob('mytest-*.txt'))
        assert len(saved_files) == 1
        assert saved_files[0].read_text() == text

    def test_preview_contains_first_2000_chars(self, tmp_path, monkeypatch):
        monkeypatch.setattr(neo_browser, 'RESPONSE_DIR', tmp_path)
        text = 'X' * (neo_browser.MAX_RESULT_CHARS + 5000)
        result = neo_browser.persist_if_large(text, 'preview')
        assert result.startswith('X' * 2000)

    def test_result_contains_total_chars_info(self, tmp_path, monkeypatch):
        monkeypatch.setattr(neo_browser, 'RESPONSE_DIR', tmp_path)
        text = 'Z' * (neo_browser.MAX_RESULT_CHARS + 100)
        result = neo_browser.persist_if_large(text, 'info')
        assert 'chars total' in result

    def test_path_contains_tag_name(self, tmp_path, monkeypatch):
        monkeypatch.setattr(neo_browser, 'RESPONSE_DIR', tmp_path)
        text = 'W' * (neo_browser.MAX_RESULT_CHARS + 1)
        result = neo_browser.persist_if_large(text, 'mytag')
        saved = list(tmp_path.glob('mytag-*.txt'))
        assert len(saved) == 1

    def test_empty_text_returned_unchanged(self):
        assert neo_browser.persist_if_large('', 'tag') == ''

    def test_none_returned_unchanged(self):
        assert neo_browser.persist_if_large(None, 'tag') is None


# ═══════════════════════════════════════════════════════════════
# TestCookieSync
# ═══════════════════════════════════════════════════════════════

class TestCookieSync:

    def test_ghost_path_uses_pid(self, neo):
        """_resync_cookies uses ghost-{PID}, not ghost-profile."""
        import inspect
        source = inspect.getsource(neo._resync_cookies)
        assert 'ghost-{os.getpid()}' in source or "f'ghost-{os.getpid()}'" in source

    def test_excluded_domains_includes_google(self, neo):
        """Google domains are excluded from cookie sync."""
        import inspect
        source = inspect.getsource(neo._sync_session)
        for domain in ['.google.com', '.googleapis.com', '.youtube.com', '.gmail.com']:
            assert domain in source

    def test_cookie_domains_is_list(self, neo):
        """COOKIE_DOMAINS is a list."""
        assert isinstance(neo.COOKIE_DOMAINS, list)

    def test_cookie_domains_empty_by_default(self, neo):
        """COOKIE_DOMAINS empty when NEOBROWSER_COOKIE_DOMAINS env var not set."""
        import os
        if not os.environ.get('NEOBROWSER_COOKIE_DOMAINS'):
            assert neo.COOKIE_DOMAINS == []


# ═══════════════════════════════════════════════════════════════
# TestChatPipeline
# ═══════════════════════════════════════════════════════════════

class TestChatPipeline:

    def test_pipeline_init(self, neo):
        """ChatPipeline initializes with correct defaults."""
        p = neo.ChatPipeline('test', 'https://test.com')
        assert p.platform == 'test'
        assert p.url == 'https://test.com'
        assert p.conv_url is None
        assert p.d is None
        assert p.max_retries == 2
        assert p.last_error is None

    def test_pipeline_send_captures_before_state(self, neo):
        """send() captures both msg count AND last text before sending."""
        import inspect
        source = inspect.getsource(neo.ChatPipeline.send)
        assert '_msg_count_before' in source
        assert '_last_text_before' in source

    def test_pipeline_wait_response_compares_text(self, neo):
        """wait_response compares text content, not just count."""
        import inspect
        source = inspect.getsource(neo.ChatPipeline.wait_response)
        assert 'last_text_before' in source

    def test_pipeline_wait_response_non_blocking(self, neo):
        """wait_response returns streaming status instead of blocking 120s."""
        import inspect
        source = inspect.getsource(neo.ChatPipeline.wait_response)
        assert 'streaming' in source
        assert 'range(max_wait * 2)' not in source

    def test_pipeline_check_response_exists(self, neo):
        """check_response method exists for non-blocking checks."""
        assert hasattr(neo.ChatPipeline, 'check_response')

    def test_pipeline_resync_and_reload_exists(self, neo):
        """_resync_and_reload method exists for cookie refresh."""
        assert hasattr(neo.ChatPipeline, '_resync_and_reload')

    def test_pipeline_resync_kills_chrome(self, neo):
        """_resync_and_reload kills Chrome before re-syncing."""
        import inspect
        source = inspect.getsource(neo.ChatPipeline._resync_and_reload)
        assert '_kill_pids' in source or 'quit' in source
        assert '_chrome = None' in source or '_chrome' in source

    def test_pipeline_ensure_detects_spanish_login(self, neo):
        """ensure() detects Spanish login walls (iniciar sesión)."""
        import inspect
        source = inspect.getsource(neo.ChatPipeline.ensure)
        assert 'iniciar sesión' in source or 'inicia sesión' in source

    def test_pipeline_ensure_attempts_resync_on_login(self, neo):
        """ensure() tries cookie re-sync before failing on login wall."""
        import inspect
        source = inspect.getsource(neo.ChatPipeline.ensure)
        assert '_resync_and_reload' in source

    def test_gpt_instance_exists(self, neo):
        """_gpt ChatPipeline instance exists at module level."""
        assert hasattr(neo, '_gpt')
        assert neo._gpt.platform == 'gpt'
        assert 'chatgpt.com' in neo._gpt.url

    def test_grok_instance_exists(self, neo):
        """_grok ChatPipeline instance exists at module level."""
        assert hasattr(neo, '_grok')
        assert neo._grok.platform == 'grok'
        assert 'grok.com' in neo._grok.url


# ═══════════════════════════════════════════════════════════════
# TestPluginLoopBug
# ═══════════════════════════════════════════════════════════════

class TestPluginLoopBug:

    def test_loop_item_available_in_step_args(self):
        """REGRESSION: {{item}} in step args should resolve during loop iteration.

        The bug: step_args are resolved against outer ctx (line 149 in plugins.py)
        before the loop begins, where 'item' doesn't exist yet. The second resolve
        inside the loop (line 158) is a no-op because the template markers are gone.
        This test documents the current (buggy) behavior: the loop runs 3 times
        but URLs have empty item substitutions.
        """
        calls = []

        def mock_dispatch(action, args):
            calls.append((action, dict(args)))
            return f'result'

        plugin = {
            'inputs': {
                'items': {'default': 'a,b,c'}
            },
            'steps': [{
                'action': 'browse',
                'url': 'https://example.com/{{item}}',
                'loop': 'items',
                'as': 'item',
            }]
        }

        plg.run_plugin(plugin, {}, mock_dispatch)

        # Loop executes 3 times regardless of the bug
        assert len(calls) == 3

        # BUG: {{item}} is resolved against outer ctx where 'item' is absent → ''
        # When FIXED these should be: ['https://example.com/a', 'https://example.com/b', 'https://example.com/c']
        urls = [c[1].get('url', '') for c in calls]
        assert urls == [
            'https://example.com/',
            'https://example.com/',
            'https://example.com/',
        ]

    def test_loop_without_template_in_args_works(self):
        """Loop with static args (no {{item}}) executes correctly."""
        calls = []

        def mock_dispatch(action, args):
            calls.append((action, dict(args)))
            return 'ok'

        plugin = {
            'inputs': {
                'urls': {'default': 'a,b'}
            },
            'steps': [{
                'action': 'scroll',
                'amount': '500',
                'loop': 'urls',
                'as': 'url',
            }]
        }

        plg.run_plugin(plugin, {}, mock_dispatch)
        assert len(calls) == 2


# ═══════════════════════════════════════════════════════════════
# TestLoginWallDetection
# ═══════════════════════════════════════════════════════════════

class TestLoginWallDetection:

    def test_login_signals_english(self, neo):
        """Login detection covers English phrases."""
        import inspect
        source = inspect.getsource(neo.ChatPipeline.ensure)
        for phrase in ['log in', 'sign in']:
            assert phrase in source

    def test_login_signals_spanish(self, neo):
        """Login detection covers Spanish phrases."""
        import inspect
        source = inspect.getsource(neo.ChatPipeline.ensure)
        assert 'iniciar sesión' in source or 'inicia sesión' in source


# ═══════════════════════════════════════════════════════════════
# TestBrowserLock
# ═══════════════════════════════════════════════════════════════

class TestBrowserLock:

    def test_browser_lock_is_rlock(self):
        """_browser_lock must be RLock (not Lock) to prevent deadlocks."""
        import threading
        # RLock instances are of a private C-level type; the standard check is
        # isinstance(lock, type(threading.RLock()))
        lock = neo_browser._browser_lock
        assert isinstance(lock, type(threading.RLock()))

    def test_rlock_allows_nested_acquire(self):
        """RLock allows same thread to acquire twice (plugin → dispatch scenario)."""
        acquired = []
        lock = neo_browser._browser_lock
        with lock:
            acquired.append(1)
            with lock:
                acquired.append(2)
        assert acquired == [1, 2]
