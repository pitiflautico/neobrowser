"""Unit tests for neo-browser.py core logic — no Chrome, no network required."""
import runpy
import sys
from pathlib import Path
import pytest

# Load neo-browser.py as a namespace (not as __main__ — avoids launching stdin loop)
_NEO_PATH = Path(__file__).parent.parent.parent / 'neo-browser.py'
_ns = runpy.run_path(str(_NEO_PATH), run_name='neo_browser_test')

validate_url = _ns['validate_url']
scan_secrets = _ns['scan_secrets']
get_mcp_tools = _ns['get_mcp_tools']
handle = _ns['handle']


# ── 1. validate_url ──────────────────────────────────────────────────────────

class TestValidateUrl:
    def test_blocks_localhost(self):
        assert validate_url('http://localhost/') is False

    def test_blocks_127_0_0_1(self):
        assert validate_url('http://127.0.0.1/') is False

    def test_blocks_private_ip_192(self):
        assert validate_url('http://192.168.1.1/') is False

    def test_blocks_private_ip_10(self):
        assert validate_url('http://10.0.0.1/') is False

    def test_allows_public_url(self):
        assert validate_url('https://example.com/') is True

    def test_blocks_file_scheme(self):
        assert validate_url('file:///etc/passwd') is False

    def test_blocks_aws_metadata(self):
        assert validate_url('http://169.254.169.254/latest/meta-data/') is False

    def test_blocks_empty_url(self):
        assert validate_url('') is False

    def test_blocks_credentials_in_url(self):
        assert validate_url('http://user:pass@example.com/') is False

    def test_allows_https_public(self):
        assert validate_url('https://anthropic.com/') is True


# ── 2. get_mcp_tools ─────────────────────────────────────────────────────────

class TestGetMcpTools:
    @pytest.fixture(scope='class')
    def tools(self):
        return get_mcp_tools()

    def test_tool_count(self, tools):
        assert len(tools) == 27

    def test_required_fields_present(self, tools):
        for t in tools:
            assert 'name' in t, f"Missing 'name' in tool {t}"
            assert 'description' in t, f"Missing 'description' in tool {t}"
            assert 'inputSchema' in t, f"Missing 'inputSchema' in tool {t}"

    def test_input_schema_type_object(self, tools):
        for t in tools:
            assert t['inputSchema']['type'] == 'object', (
                f"inputSchema.type is not 'object' for tool '{t['name']}'"
            )

    def test_scroll_amount_is_integer(self, tools):
        scroll = next(t for t in tools if t['name'] == 'scroll')
        amount_type = scroll['inputSchema']['properties'].get('amount', {}).get('type')
        assert amount_type == 'integer', f"scroll.amount type should be 'integer', got {amount_type!r}"

    def test_scroll_direction_enum(self, tools):
        scroll = next(t for t in tools if t['name'] == 'scroll')
        direction_enum = scroll['inputSchema']['properties'].get('direction', {}).get('enum')
        assert direction_enum is not None, "scroll.direction should have an enum"
        assert 'up' in direction_enum
        assert 'down' in direction_enum

    def test_all_tools_have_unique_names(self, tools):
        names = [t['name'] for t in tools]
        assert len(names) == len(set(names)), "Duplicate tool names found"


# ── 3. MCP protocol handler (tools/list) ─────────────────────────────────────

class TestMcpHandler:
    def test_tools_list_response(self, capsys):
        import json
        req = {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/list', 'params': {}}
        handle(req)
        captured = capsys.readouterr()
        response = json.loads(captured.out.strip())
        assert response['jsonrpc'] == '2.0'
        assert response['id'] == 1
        assert 'result' in response
        tools = response['result']['tools']
        assert len(tools) == 27

    def test_initialize_response(self, capsys):
        import json
        req = {'jsonrpc': '2.0', 'id': 2, 'method': 'initialize', 'params': {}}
        handle(req)
        captured = capsys.readouterr()
        response = json.loads(captured.out.strip())
        assert response['id'] == 2
        result = response['result']
        assert result['protocolVersion'] == '2024-11-05'
        assert 'tools' in result['capabilities']

    def test_unknown_method_returns_error(self, capsys):
        import json
        req = {'jsonrpc': '2.0', 'id': 3, 'method': 'unknown/method', 'params': {}}
        handle(req)
        captured = capsys.readouterr()
        response = json.loads(captured.out.strip())
        assert response['id'] == 3
        assert 'error' in response

    def test_unknown_tool_returns_error(self, capsys):
        import json
        req = {
            'jsonrpc': '2.0', 'id': 4, 'method': 'tools/call',
            'params': {'name': 'nonexistent_tool', 'arguments': {}}
        }
        handle(req)
        captured = capsys.readouterr()
        response = json.loads(captured.out.strip())
        assert response['id'] == 4
        assert 'error' in response


# ── 4. scan_secrets ───────────────────────────────────────────────────────────

class TestScanSecrets:
    def test_detects_anthropic_key(self):
        # Pattern: sk-ant-api\w{20,}  (\w = [a-zA-Z0-9_], no dash after prefix)
        text = 'Here is the key: sk-ant-api03xyz123abc456def789ghi012'
        result = scan_secrets(text)
        assert len(result) > 0, "Should detect Anthropic API key"
        assert any('Anthropic' in r for r in result)

    def test_clean_text_returns_empty(self):
        text = 'This is normal text without any secrets'
        result = scan_secrets(text)
        assert result == []

    def test_detects_openai_key(self):
        # Pattern: sk-[a-zA-Z0-9]{20,}  (20+ alphanumeric immediately after 'sk-')
        text = 'key = "sk-abc123def456ghi789jkl012mnopqr"'
        result = scan_secrets(text)
        assert len(result) > 0, "Should detect OpenAI API key"

    def test_detects_aws_access_key(self):
        text = 'AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE'
        result = scan_secrets(text)
        assert len(result) > 0, "Should detect AWS Access Key"
        assert any('AWS' in r for r in result)

    def test_detects_github_pat(self):
        text = 'token: ghp_' + 'a' * 36
        result = scan_secrets(text)
        assert len(result) > 0, "Should detect GitHub PAT"

    def test_detects_private_key_header(self):
        text = '-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA...'
        result = scan_secrets(text)
        assert len(result) > 0, "Should detect Private Key"

    def test_empty_text_returns_empty(self):
        assert scan_secrets('') == []

    def test_none_returns_empty(self):
        assert scan_secrets(None) == []
