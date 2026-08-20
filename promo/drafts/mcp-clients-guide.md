# NeoBrowser — Guía de integración con clientes MCP

> One-liner: configura NeoBrowser en Claude Code, Cursor, VS Code, Claude Desktop o Windsurf en menos de 60 segundos.

## Instalación previa

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/pitiflautico/neobrowser/main/install.sh | sh

# Windows: descarga el binario desde Releases
# https://github.com/pitiflautico/neobrowser/releases/latest
```

Verifica que `neobrowser` está en PATH:

```bash
neobrowser doctor
```

## Configuración básica

La configuración mínima para cualquier cliente MCP:

```json
{
  "mcpServers": {
    "neobrowser": {
      "command": "neobrowser"
    }
  }
}
```

## Clientes

### Claude Code

```bash
claude mcp add neobrowser -- neobrowser
```

O edita `~/.claude/mcp.json`:

```json
{
  "mcpServers": {
    "neobrowser": {
      "command": "neobrowser"
    }
  }
}
```

### Claude Desktop

Edita `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS):

```json
{
  "mcpServers": {
    "neobrowser": {
      "command": "neobrowser"
    }
  }
}
```

Reinicia Claude Desktop.

### Cursor

Ve a **Cursor Settings → MCP → Add new MCP server**:

- Name: `neobrowser`
- Type: `command`
- Command: `neobrowser`

O añade a `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "neobrowser": {
      "command": "neobrowser"
    }
  }
}
```

### VS Code

VS Code 1.99+ soporta MCP en Agent Mode. Añade a tu `settings.json`:

```json
{
  "mcp": {
    "inputs": [],
    "servers": {
      "neobrowser": {
        "command": "neobrowser",
        "type": "stdio"
      }
    }
  }
}
```

Badge de instalación 1-click ya en README.

### Windsurf

Edita `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "neobrowser": {
      "command": "neobrowser"
    }
  }
}
```

## Modo sesión real (opcional, opt-in)

Para que el agente use tu Chrome real con tus sesiones logueadas:

```json
{
  "mcpServers": {
    "neobrowser": {
      "command": "neobrowser",
      "env": {
        "NEOBROWSER_REAL_PROFILE": "Default"
      }
    }
  }
}
```

- macOS: descifra cookies del Keychain.
- Linux: descifra via secret-service.
- Windows: descifra via DPAPI.

Identity cookies para Google/LinkedIn/Microsoft están excluidas para no desloguear tu navegador real.

## Primer comando de prueba

En cualquier cliente, prueba:

```
navigate to https://example.com and read the page
```

Si todo va bien, el modelo llamará a `navigate` y `read` y verás el contenido de la página.

## Solución de problemas

| Síntoma | Causa probable | Fix |
|---|---|---|
| `neobrowser: command not found` | No está en PATH | Añade `~/.local/bin` o `/usr/local/bin` a PATH |
| Chrome no se encuentra | `NEOBROWSER_CHROME_BIN` no definido | `export NEOBROWSER_CHROME_BIN=/Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome` |
| Timeout en primera navegación | Chrome arranca lento | Normal en frío; reintentar |
| `ProfileInUse` | Chrome ya corre con ese perfil | Usa `NEOBROWSER_PROFILE=otro` o `NEOBROWSER_ATTACH_PORT=9222` |

---

*Versión corta para posts sociales:*

```
Add NeoBrowser to Claude Code in one line:

claude mcp add neobrowser -- neobrowser

It drives your real Chrome with your real sessions. MIT, single binary.
https://github.com/pitiflautico/neobrowser
```
