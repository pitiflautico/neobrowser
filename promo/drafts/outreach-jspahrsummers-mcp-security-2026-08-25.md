# Outreach — Justin Spahr-Summers (@jspahrsummers, MCP spec lead)

## Ángulo
Pregunta técnica sobre seguridad y contratos de capabilities en MCP, desde la experiencia construyendo un browser server.

## Vía
X reply/DM o issue de discusión en github.com/modelcontextprotocol/specification. Preferir issue pública si el tema es genuinamente spec-level.

## Draft

```
Hi Justin,

I'm working on NeoBrowser, an open-source MCP server that drives the user's real Chrome over CDP (repo: https://github.com/pitiflautico/neobrowser).

One thing we've struggled with: the MCP spec is intentionally minimal about what a "browser tool" server must guarantee. We ended up building our own layers for origin-scoped credentials, human-approval gates, audit logs, bot-wall detection, and verified actions. It works, but every client has to inspect our tool descriptions to discover those safety properties.

Have you considered a capability/extension mechanism where a server can advertise higher-level contracts like "browser-automation" with declared safety invariants? Or is the preference to keep the spec small and let conventions emerge?

Happy to open a discussion issue if this is the right venue.
```

## Por qué funciona
- Pregunta legítima de spec design.
- Muestra trabajo propio sin vender.
- Ofrece un siguiente paso concreto (abrir issue).

## Estado
Borrador listo. Sin cuenta/ sesión activa de X para envío automático; publicación manual o issue de discusión pendiente.
