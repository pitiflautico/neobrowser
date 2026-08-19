# Decisión: mergear el pre-launch hardening (#6 / #7)

## Estado actual

- **PR #6**: `t-soriano-sesame/neobrowser` → `pitiflautico/neobrowser:main`  
  Título: *Pre-launch hardening: verified-action contract, security fixes, and a Chrome input fault worth a second look*  
  +27.231 / -6.149 líneas, 15 commits.  
  **mergeStateStatus: `DIRTY`** → tiene conflictos con `main`, no se puede mergear directamente.

- **PR #7**: `pitiflautico/neobrowser` (branch interna) → `main`  
  Título: *Pre-launch hardening (PR #6) — validated and integrated with main*  
  +27.274 / -6.199 líneas, 24 commits (integra #6 + merge de `origin/main` + promo commits).  
  **mergeStateStatus: `DIRTY`** → también tiene conflictos, probablemente porque `main` ha avanzado después de que se creó.

Ninguno tiene review Decision (están sin revisar formalmente).

---

## Qué traen (resumen ejecutivo)

### 1. Verified-action contract (`docs/VERIFIED-ACTIONS.md`)
- Especifica 6 estados, 10 invariantes y 13 escenarios de conformidad.
- Antes: `click` devolvía `Clicked` siempre, aunque no pasara nada.
- Ahora: cada acción mutante informa un estado derivado de una observación antes, una después y la diferencia detectada.
- Tests de conformidad ejecutables: `cargo test --test conformance`, 13/13 pasando contra Chrome real.

### 2. Seguridad
- Sandbox: `--no-sandbox` ya no es incondicional.
- Validación de orígenes: `http://localhost.evil.test` ya no pasa como localhost.
- Matching de host: `notgoogle.com` ya no coincide con `google.com`.
- Credenciales en redirecciones: headers `Authorization` no se reenvían automáticamente en 302.
- TOCTOU en upload/download: copia segura de archivos.
- Extensión Chrome (`extension/`, manifest v3, permiso `debugger`) para herramientas `bridge_*`.

### 3. Robustez real
- `find_and_click` ahora busca la etiqueta visible (textContent, aria-label, value, title, alt, placeholder) y acepta `input[type=button/image/reset/submit]`.
- Quiescencia antes de la acción: baseline estable; atribución del cambio solo a la acción que lo causó.
- Manejo de un fault de Chrome 151 donde `Input.dispatchMouseEvent` se acepta pero no se entrega: detecta y reemplaza la pestaña.

### 4. Refactor masivo
- `main.rs` y `mcp.rs` divididos; todo el crate en módulos < 250 líneas.
- JS embebido migrado a `rust/js/` como snippets reales.
- Layout moderno (`src/page.rs` junto a `src/page/eval.rs`), sin `mod.rs`.
- Python oracle movido a `archive/python-oracle/`.

### 5. CI/documentación
- Plantillas de issues, `CONTRIBUTING.md`, `SECURITY.md`, `docs/REPRODUCIBILITY.md`.
- Workflows propuestos en `.github/workflows-proposed/` (no aplicados porque el fork no tiene token `workflow`).
- `scripts/check-tracked.py` para evitar que `.gitignore` se coma archivos fuente.

---

## Verificación reportada por el autor

- Build limpio en macOS.
- 324 tests verdes, incluyendo 13 de conformidad contra Chrome real.
- `cargo fmt --check`, clippy, rustdoc bajo `-D warnings`, cargo audit: limpio.
- Catálogo de herramientas: 43 → 67.
- CI del fork no corría ("no checks"); por eso existe #7 para validar en Linux.

---

## Recomendación

**Mergear #7, no #6.**

#7 es la versión integrada con `main` y con los promo commits. #6 es el fork original y ya está desactualizado.

**Paso previo obligatorio:** resolver conflictos con `main` actual. Ambos PRs están `DIRTY`.

### Opciones para resolver

1. **Rebase de #7 sobre `main` actual** (preferido si la historia está limpia):
   ```bash
   gh repo clone pitiflautico/neobrowser /tmp/nbmerge
   cd /tmp/nbmerge
   gh pr checkout 7
   git fetch origin main
   git rebase origin/main
   # resolver conflictos
   git push --force-with-lease
   ```

2. **Merge de `main` en #7** (más seguro si hay muchos commits):
   ```bash
   gh pr checkout 7
   git fetch origin main
   git merge origin/main
   # resolver conflictos
   git push
   ```

Tras limpiar el estado, mergear con merge commit para preservar la historia del hardening:
```bash
gh pr merge 7 --merge --subject "Pre-launch hardening: verified-action contract, security fixes, and Chrome input fault handling"
```

### Riesgos a considerar

- Es un cambio enorme (+27k/-6k). Aunque el autor reporta tests verdes, conviene correr `cargo test` localmente después del rebase.
- La extensión Chrome (`extension/`) es un nuevo artefacto con superficie de mantenimiento.
- Los workflows propuestos no se aplican automáticamente; hay que moverlos manualmente de `.github/workflows-proposed/` a `.github/workflows/` si se quieren activar.
- El cambio de tool catalogue (43 → 67) puede afectar la documentación `docs/TOOLS.md` y la landing; revisar que sigan consistentes.

### Beneficio de mergear ahora

- Cierra el canal de "falsos éxitos" en clicks antes del lanzamiento.
- Da una base de seguridad sólida para Product Hunt y el registro MCP.
- Permite contar la historia "testeado contra Chrome real con contrato verificado" en el marketing.

---

## Decisión pendiente

No mergear hasta que el usuario dé el visto bueno o hasta que se resuelvan los conflictos. Este documento sirve como input para esa decisión.
