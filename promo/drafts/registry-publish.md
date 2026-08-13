# Publicar en el MCP Registry oficial — instrucciones (1 minuto de OAuth)

Todo está preparado: `server.json` ya está en la raíz del repo (válido según el schema 2025-12-11, variante "custom installation path" con `websiteUrl`, ya que NeoBrowser se distribuye como binario por GitHub Releases, no por npm/pypi). El nombre `io.github.pitiflautico/neobrowser` se verifica contra tu cuenta de GitHub al hacer login.

## Pasos (solo la primera vez)

```bash
# 1. Instalar el publisher (elige uno):
brew install mcp-publisher        # o descarga el binario de github.com/modelcontextprotocol/registry/releases

# 2. Login con GitHub (abre el navegador, autoriza, listo):
mcp-publisher login github

# 3. Publicar (desde la raíz del repo):
mcp-publisher publish
```

Eso es todo. Al publicar, el listing propaga a los agregadores downstream que ingieren el registry.

## Al subir de versión

1. Actualizar `version` en `server.json` y en `rust/Cargo.toml` (mantenerlas sincronizadas).
2. `mcp-publisher publish` de nuevo.

## Si algún día hay paquete npm/cargo

Añadir entrada en `packages[]` con `registryType: "npm"` (o `"cargo"`) — ver ejemplos en
https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/server-json/generic-server-json.md
