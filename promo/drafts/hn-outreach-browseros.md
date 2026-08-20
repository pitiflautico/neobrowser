# Borrador comentario HN — BrowserOS (45618536)

**Contexto**: post "Show HN: We packaged an MCP server inside Chromium" (46 pts, 17 comments). BrowserOS es un fork de Chromium con MCP server embebido; destacan sesiones logueadas y APIs nativas de interacción.

**Tono**: técnico, respetuoso, aportar perspectiva. Mencionar NeoBrowser solo al final.

---

```
The "logged-in sessions" point is the real differentiator. chrome-devtools-mcp starts fresh because CDP defaults to a new user-data-dir, but it can attach to a running Chrome — it's just not the happy path.

We've been exploring the opposite extreme: instead of shipping another Chromium fork, drive the user's already-installed Chrome via CDP and reuse their real profile/sessions as opt-in. The advantage is you don't ask users to switch browsers; the disadvantage is you inherit all of Chrome's quirks and the security model becomes "we read your cookies only when you say so."

One thing BrowserOS gets right that I wish CDP exposed more cleanly: drawing bounding boxes and direct interaction APIs. CDP's accessibility tree is a firehose, and most agents don't need 90% of it.

(Side note: also playing in this space with github.com/pitiflautico/neobrowser, same "use real Chrome" angle.)
```

## Reglas de publicación
- Publicar solo si el hilo sige activo y el comentario aporta a la discusión.
- 80% valor, 20% mención honesta.
- Publicar manualmente desde cuenta HN del usuario.
