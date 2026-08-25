# Outreach — Alex Albert (@alexalbert__, Anthropic dev rel)

## Ángulo
Feedback genuino sobre MCP + browser automation desde las trincheras. No pedir RT, pedir opinión técnica.

## Vía
X DM o reply a un post sobre MCP / tool use / agents. Si X sigue bloqueado, dejar borrador para publicación manual del usuario.

## Draft

```
hey Alex — quick question from someone deep in the MCP browser-automation weeds.

We've been building NeoBrowser, an MCP server that drives the user's *real* Chrome over CDP instead of launching another headless browser. The idea is that agents inherit the user's actual trust state (cookies, fingerprint, sandbox) rather than faking it.

The part I'd love your take on: MCP currently leaves a lot of browser-tool semantics to each server (approval gates, origin scoping, audit trails, bot-wall detection). We're implementing all of them, but it feels like the spec could eventually benefit from a shared "browser capability" contract so clients know what safety properties a server claims.

Is that something Anthropic's MCP team has appetite for, or is the deliberate choice to keep the spec minimal here?

Repo + benchmark: https://github.com/pitiflautico/neobrowser

No rush — genuinely curious.
```

## Por qué funciona
- No pide promoción.
- Plantea una pregunta técnica concreta (capability contract para browser tools).
- Muestra que conocemos el spec y sus límites.
- Link al repo solo al final, como referencia.

## Estado
Borrador listo. X bloqueado por CAPTCHA en automatización; publicación manual pendiente.
