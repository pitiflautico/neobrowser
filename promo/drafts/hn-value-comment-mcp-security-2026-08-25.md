# Hacker News value comment — MCP security thread

## Thread objetivo
Cualquier thread reciente sobre seguridad en MCP, por ejemplo:
- "Supabase MCP can leak your entire SQL database" (44502318)
- "The 'S' in MCP Stands for Security" (43600192)
- "A critical look at MCP" (43945993)

## Draft comment

```
One design pattern that helps is treating the MCP server as a sandboxed
extension of the browser, not as a privileged client.

We built our browser MCP server around a few rules:

- Credentials are origin-scoped. A cookie for example.com never leaves
  example.com, even if the LLM asks for it.
- Every mutating action returns a verified envelope: before/after state,
  so the agent can't claim it clicked something it didn't.
- Human-approval gates for cross-origin or high-risk actions.
- Renderer sandbox stays on; no --no-sandbox as a default.
- Session identity cookies for Google/GitHub/etc. are never cloned from
  the user's real profile, because providers revoke duplicate sessions.

It doesn't eliminate risk, but it moves the failure mode from "the LLM
accidentally exfiltrated data" to "the LLM asked for something and the
policy refused." That's a much easier thing to audit.

Repo: https://github.com/pitiflautico/neobrowser
```

## Por qué es seguro para HN
- No es autopromoción descarada; responde directamente al tema del thread.
- El link al repo va al final, como referencia.
- Enseña algo concreto (origin-scoped credentials, verified envelope).
- No suena a marketing.

## Estado
Borrador listo. HN posts propios están pausados por el flag anterior, pero los
comentarios value-first en threads ajenos son aceptables. Publicación manual
pendiente o vía NeoBrowser con supervisión.
