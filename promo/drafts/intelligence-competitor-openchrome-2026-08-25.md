# Inteligencia de competencia — OpenChrome (shaun0927/openchrome)

## Ficha

- **Repo:** https://github.com/shaun0927/openchrome
- **Estrellas:** 234★ (actualizado 2026-08-23)
- **Lenguaje:** TypeScript / Node.js (publicado en npm como `openchrome-mcp`)
- **Claim principal:** "The MCP server that drives and guides AI agents through a real Chrome."
- **Mascota:** Raptor (muy memorable, ya tiene asset visual propio).

## Qué hace bien

1. **Mensaje ultra claro:** una frase, un ejemplo concreto (comparar precios en 4 tiendas), una tabla de comparación dramática.
2. **Paralelismo como feature central:** "4 parallel lanes, already authenticated everywhere" suena a superpoder.
3. **Métricas agresivas:** ~3s vs ~250s, 300 MB vs 2.5 GB. Aunque sean casos de uso seleccionados, venden.
4. **Múltiples puntos de entrada:** npm global, desktop app, daemon HTTP, CLI directo (`oc run`), playbooks YAML.
5. **118 herramientas** (vs nuestras 67) organizadas en capability map.
6. **Conceptos de marketing propios:** "harness-engineered", "lanes", "Outcome Contract", "circuit breaker".
7. **Localización:** README en inglés y coreano.
8. **Desktop app beta:** reduce fricción para no desarrolladores.

## Diferenciadores que aún tenemos

- **Rust nativo:** un solo binario ~6 MB, no requiere Node.js/npm. OpenChrome depende de npm o un standalone opcional.
- **Benchmark honesto publicado:** nosotros comparamos contra Playwright MCP y admitimos dónde perdemos. OpenChrome solo muestra una tabla favorable sin metodología visible.
- **Seguridad first-class:** domain rules, approval gates, audit log, vault cifrado, sandbox por defecto. OpenChrome no destaca esto en su README principal.
- **Anti-detection "genuine, not spoofed":** heredamos fingerprint real; no afirmamos "invisible" (que es claim difícil de defender).
- **Cross-platform cookie decryption:** macOS Keychain, Linux secret-service, Windows DPAPI.

## Tácticas aplicables YA

1. **Mascota / asset visual propio:** OpenChrome tiene el Raptor. Nosotros solo tenemos el logo cuadrado. Una mascota aumenta shareability.
2. **README hero con ejemplo concreto y tabla:** copiar la estructura (1 ejemplo + 1 tabla) pero con datos reales del benchmark.
3. **Playbooks YAML visibles:** ya tenemos playbooks, pero no los mostramos en el README hero.
4. **Comando one-liner de instalación:** `cargo install --git ...` es bueno, pero añadir un install script (`curl ... | sh`) bajaría fricción.
5. **Desktop app / daemon HTTP:** son roadmap, pero anunciarlos en README/landing como "coming soon" genera expectativa.
6. **Localizar README:** al menos añadir español (mercado natural del founder) y coreano/chino si queremos Asia.
7. **Número de herramientas en el tagline:** "67 tools" ya lo decimos, pero "118 tools" de OpenChrome suena más. Podemos contar sub-tools o crear más tools pequeñas para llegar a 100+.

## Amenaza real

OpenChrome está creciendo rápido (234★ en poco tiempo) y su mensaje es casi idéntico al nuestro. Si no aceleramos distribución y contenido, nos pueden comer el nicho de "real Chrome MCP server".

## Recomendación inmediata

- Publicar un post comparativo honesto: "NeoBrowser vs OpenChrome" — destacar Rust/binario único, seguridad, benchmark reproducible.
- Lanzar Product Hunt cuanto antes para captar tráfico de "real browser MCP".
- Crear una mascota o personaje para las publicaciones virales (GIFs con el contador de estrellas).
