# PDR: Chrome Transport — Arquitectura definitiva para Cloudflare

## Fecha: 25 March 2026

## El problema real

Hemos pasado 2 sesiones parchando TLS fingerprints (wreq → impit → Chrome CDP fallback). Nada funciona porque:

1. **wreq**: Cloudflare detecta que no es Chrome real (JA3/JA4/HTTP2 SETTINGS)
2. **impit**: Chrome 142 fingerprint tampoco pasa (Cloudflare actualiza faster que nosotros)
3. **Chrome CDP fallback**: Lanza Chrome nuevo en about:blank → sin cookies → sin contexto → 403
4. **Buscar Chrome existente**: Hackear DevToolsActivePort files no funciona — neobrowser usa su propio Chrome con port discovery interno

El approach reactivo (detectar 403 → retry con otro client) es lento, frágil, y rompe el event loop de JS.

## La solución

### Principio: No competir con Chrome. Usar Chrome.

```
NeoRender V2 (cerebro)           Chrome real (transporte)
┌─────────────────────┐          ┌──────────────────────┐
│ V8 + happy-dom      │          │ Neobrowser Chrome     │
│ React hydration     │◄────────►│ TLS real              │
│ Events + DOM        │  HTTP    │ Cookies reales        │
│ WOM extraction      │  proxy   │ Cloudflare ✓          │
│ MCP tools           │          │ Sessions activas      │
└─────────────────────┘          └──────────────────────┘
```

NeoRender V2 hace TODO excepto las HTTP requests a dominios protegidos por Cloudflare. Para esos, delega a Chrome via neobrowser MCP.

### Detección: ¿Cuándo usar Chrome transport?

**Al navegar** (en `browse(url)` o `navigate(url)`), ANTES de ejecutar scripts:

```rust
fn detect_cloudflare(response: &HttpResponse) -> bool {
    let headers = &response.headers;
    // Cloudflare siempre envía cf-ray
    headers.contains_key("cf-ray")
    || headers.get("server").map(|v| v.contains("cloudflare")).unwrap_or(false)
    || headers.contains_key("cf-cache-status")
    || headers.contains_key("cf-mitigated")
}
```

Si detectamos Cloudflare:
1. Marcar el dominio en un `HashSet<String>` → `cloudflare_domains`
2. Todos los `op_fetch` y `op_fetch_start` para ese dominio van por Chrome transport
3. La navegación inicial ya funcionó (wreq pasa para HTML, solo las API calls fallan)

### Transporte: ¿Cómo hablar con Chrome?

**No lanzar Chrome. No buscar ports. Usar neobrowser MCP.**

Neobrowser ya tiene Chrome corriendo con sesiones activas. Exponemos un nuevo tool o usamos `browser_fetch`:

#### Opción A: Usar `browser_fetch` de neobrowser (ya existe)

```
neoV2 op_fetch detecta Cloudflare domain
  → llama a neobrowser MCP browser_fetch(url, method, headers, body)
  → neobrowser ejecuta fetch via su Chrome
  → devuelve (status, headers, body) a neoV2
```

Problema: neoV2 no puede llamar a neobrowser MCP directamente (son MCPs paralelos, no hay inter-MCP communication en Claude Code).

#### Opción B: Neobrowser expone HTTP proxy

Neobrowser levanta un HTTP proxy local (ej: `localhost:9876`):
```
POST http://localhost:9876/fetch
{
  "url": "https://chatgpt.com/backend-api/f/conversation",
  "method": "POST",
  "headers": {...},
  "body": "..."
}
→ Response: { "status": 200, "headers": {...}, "body": "..." }
```

neoV2 hace requests a ese proxy en vez de wreq para dominios Cloudflare.

**Este es el approach correcto.** Es simple, rápido, sin hacks.

#### Opción C: CDP directo via WebSocket compartido

Neobrowser escribe su CDP WebSocket URL en un file conocido:
```
~/.neobrowser/cdp.json
{
  "ws": "ws://127.0.0.1:12345/devtools/browser/abc-123",
  "port": 12345
}
```

neoV2 lee ese file y se conecta directamente via CDP.

**Más complejo que B pero más flexible** — permite buscar tabs, ejecutar JS, etc.

### Recomendación: Opción B (HTTP proxy)

Razones:
1. **Simple**: un endpoint HTTP, sin WebSocket, sin CDP
2. **Rápido**: fetch local es <1ms overhead
3. **Desacoplado**: neoV2 no necesita saber nada de CDP
4. **Testeable**: curl puede verificar el proxy
5. **Futuro**: cualquier otro tool puede usar el proxy

### Implementación

#### Paso 1: Neobrowser HTTP proxy (en neobrowser)

Archivo: `neobrowser/mcp-server/src/fetch-proxy.ts`

```typescript
import { createServer } from 'http';

// Start a local HTTP proxy that routes through Chrome
const server = createServer(async (req, res) => {
  const body = await readBody(req);
  const { url, method, headers, body: fetchBody } = JSON.parse(body);

  // Execute fetch in Chrome's context via CDP
  const result = await page.evaluate(async (args) => {
    const resp = await fetch(args.url, {
      method: args.method,
      headers: args.headers,
      body: args.body,
      credentials: 'include',
    });
    const text = await resp.text();
    const h = {};
    resp.headers.forEach((v, k) => { h[k] = v; });
    return { status: resp.status, headers: h, body: text };
  }, { url, method, headers, body: fetchBody });

  res.writeHead(200, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify(result));
});

server.listen(9876);
console.log('Chrome fetch proxy on :9876');
```

#### Paso 2: neoV2 Chrome transport client

Archivo: `crates/neo-http/src/chrome_transport.rs`

```rust
pub struct ChromeTransport {
    proxy_url: String, // http://localhost:9876/fetch
}

impl ChromeTransport {
    pub fn new(port: u16) -> Self {
        Self { proxy_url: format!("http://127.0.0.1:{port}/fetch") }
    }

    pub async fn fetch(
        &self, url: &str, method: &str,
        headers: &HashMap<String, String>, body: Option<&str>
    ) -> Result<(u16, HashMap<String, String>, String), String> {
        // Simple HTTP POST to local proxy
        let payload = json!({ "url": url, "method": method, "headers": headers, "body": body });
        let resp = wreq::Client::new()
            .post(&self.proxy_url)
            .json(&payload)
            .send().await?;
        let result: FetchResult = resp.json().await?;
        Ok((result.status, result.headers, result.body))
    }
}
```

#### Paso 3: Integración en op_fetch

```rust
// En op_fetch, para dominios Cloudflare:
if cloudflare_domains.contains(&domain) {
    if let Some(transport) = state.try_borrow::<ChromeTransport>() {
        return transport.fetch(url, method, &headers, body.as_deref()).await;
    }
}
// Fallback: wreq normal
```

#### Paso 4: Detección automática

```rust
// En browser_impl.rs navigate(), después de HTTP fetch:
if detect_cloudflare(&response) {
    self.cloudflare_domains.insert(domain.clone());
    eprintln!("[session] Cloudflare detected for {domain} — using Chrome transport");
}
```

### Chrome version sync

Leer versión de Chrome real al arrancar:

```rust
fn detect_chrome_version() -> String {
    let output = Command::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        .arg("--version")
        .output();
    // Parse "Google Chrome 146.0.7680.154" → "146"
    // Set USER_AGENT, SEC_CH_UA dinámicamente
}
```

Hoy tenemos hardcoded Chrome 145. Chrome real es 146. Este mismatch es detectable.

### Plan de ejecución

| Paso | Qué | Dónde | Tiempo |
|---|---|---|---|
| 1 | Detección Cloudflare en navigate | `browser_impl.rs` | 30 min |
| 2 | HTTP proxy en neobrowser | `neobrowser/fetch-proxy.ts` | 1 hora |
| 3 | ChromeTransport client en neoV2 | `neo-http/chrome_transport.rs` | 30 min |
| 4 | Integración en op_fetch | `ops.rs` | 30 min |
| 5 | Chrome version sync | `headers.rs` + `main.rs` | 30 min |
| 6 | Test end-to-end | ChatGPT PONG | 30 min |

**Total: 1 sesión.**

### Qué eliminamos

Con este approach, eliminamos:
- `crates/neo-chrome/src/fetch_proxy.rs` (Chrome CDP proxy directo) → reemplazado por HTTP proxy
- `crates/neo-runtime/src/chrome_fallback.rs` (reactive fallback) → reemplazado por proactive detection
- impit dependency → no necesario si Chrome transport funciona
- `find` command para buscar ports → no necesario
- Scan de DevToolsActivePort → no necesario

### Diagrama de flujo

```
browse("https://chatgpt.com")
  │
  ├── HTTP fetch (wreq) → HTML response
  │     └── Headers: cf-ray: abc123 → CLOUDFLARE DETECTED
  │
  ├── Mark domain "chatgpt.com" → cloudflare_domains
  │
  ├── Parse HTML, bootstrap, execute scripts
  │     │
  │     └── JS fetch("/backend-api/conversations")
  │           │
  │           ├── Domain in cloudflare_domains?
  │           │   YES → ChromeTransport.fetch(url) → proxy localhost:9876
  │           │          → neobrowser Chrome executes real fetch
  │           │          → response back to JS
  │           │
  │           │   NO → wreq.fetch(url) (normal path)
  │           │
  │           └── Response to JS → React updates → DOM changes
  │
  └── Extract WOM → return to AI
```

### Métricas de éxito

| Test | Hoy | Target |
|---|---|---|
| ChatGPT navigate | ✅ | ✅ |
| ChatGPT API calls (/me, /models) | ✅ (wreq funciona) | ✅ |
| ChatGPT /f/conversation (PONG) | ❌ 403 | ✅ 200 via Chrome transport |
| Factorial navigate | ✅ | ✅ |
| Factorial API calls | ❌ (error page) | ✅ via Chrome transport |
| Sesame (no Cloudflare) | ✅ wreq directo | ✅ wreq directo (sin proxy) |
| Detección Cloudflare | ❌ no existe | ✅ automática por cf-ray header |
| Chrome version match | ❌ hardcoded 145 | ✅ detectado de Chrome real |
