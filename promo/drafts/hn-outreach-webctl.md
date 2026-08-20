# Borrador comentario HN — Webctl (46616481)

**Contexto**: post "Show HN: Webctl – Browser automation for agents based on CLI instead of MCP" (134 pts, 38 comments). El autor destaca persistencia de sesión/cookies para SSO como motivación principal.

**Tono**: first-person, técnico, aportar valor. Mencionar NeoBrowser solo al final como side note.

---

```
This CLI-first approach resonates. I spent weeks fighting the opposite problem: MCP servers that dump the full accessibility tree into context and still fail the moment a site asks for a login.

The session persistence point is the real killer feature here. Keeping a daemon alive so cookies/SSO survive across commands is something every serious browser automation tool eventually has to solve. We ended up driving the user's real Chrome via CDP and decrypting cookies platform-locally (Keychain/secret-service/DPAPI) as opt-in, but a persistent daemon with a clean CLI contract is a valid alternative.

One suggestion: if you can make webctl attach to an existing Chrome profile instead of always launching its own, you'll skip a whole class of "why did my SSO log out" support issues.

(Side note: also playing in this space with github.com/pitiflautico/neobrowser, but I mean the above regardless — session state is the hardest honest problem in browser automation.)
```

## Reglas de publicación
- Solo publicar si el hilo sigue activo y el comentario encaja naturalmente.
- No publicar si hay riesgo de parecer promocional; este borrador es 80% valor, 20% mención honesta.
- Publicar manualmente desde cuenta HN del usuario.
