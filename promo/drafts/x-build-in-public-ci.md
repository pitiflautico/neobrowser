# Borrador X — build in public: desbloqueando el CI

**Asset:** GIF del contador de estrellas en la landing o screenshot del CI verde
**Cuándo publicar:** después de que PR #7 se mergee y CI esté verde
**Tono:** first-person, founder, aprender en público

---

**Opción A — corto y directo**

```
PR #7 era un monstruo: verified-action contract, seguridad, refactor de 15 módulos.

Cada push descubría un nuevo fallo de CI:
- README decía 28 vars, el binario leía 31
- gitleaks detectaba una key de test que ya estaba allowlistada
- macOS/Windows morían en los tests de conformance porque Chrome sin sandbox tarda el triple

Ahora el CI pasa en los 3 sistemas. El pre-launch hardening ya puede mergearse.

88★ → 10k. Cada estrella me mantiene encendido.

→ github.com/pitiflautico/neobrowser
```

**Opción B — con stakes**

```
Mi "empleado de IA" tiene una sola KPI: llevar NeoBrowser a 10.000 estrellas.

Hoy su trabajo ha sido arreglar CI, no hype:
- documentar 3 env vars que el README ocultaba
- reemplazar gitleaks-action por el CLI directo (la action enmascaraba fallos reales)
- subir timeouts en macOS/Windows para que los tests de conformance no mueran

El PR #7 ya es mergeable. Siguiente fase: contenido, outreach, Product Hunt.

Si crees que un navegador real para agentes de IA es útil, una estrella ayuda más de lo que parece.

→ github.com/pitiflautico/neobrowser
```

## Notas
- No adjuntar enlace en el primer tweet; el link al repo va al final.
- Si hay GIF del CI verde, adjuntarlo.
- Responder a comentarios técnicos con detalles del verified-action contract.
