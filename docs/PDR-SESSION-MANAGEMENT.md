# PDR: Session & Tab Management — NeoBrowser V3

## Problema

Ghost Chrome headless controla webs con sesión del usuario (X, ChatGPT, LinkedIn).
La gestión actual de tabs + cookies + navegación es frágil:

- `Target.createTarget` crea tabs que no navegan correctamente
- `Page.navigate` en tabs nuevas a veces no dispara
- Las cookies están en el profile pero Chrome no siempre las envía en tabs recién creadas
- No hay tracking de estado de cada tab (URL, loaded, error)
- ChatGPT pierde su WebSocket interno al navegar fuera y volver

## Contexto técnico

### Qué funciona
- Tab default: `open("https://x.com/karpathy")` → carga con sesión ✅
- Tab default: `open("https://chatgpt.com")` → carga con sesión ✅
- Tab default: `open("https://linkedin.com/feed")` → carga con sesión ✅
- Smart fill, extract, scroll, read — todo funciona en tab default ✅
- SSE interceptor captura respuestas de ChatGPT via TransformStream ✅

### Qué falla
- `Target.createTarget` + `Page.navigate` → tab se queda en about:blank
- ChatGPT en tab separada: pierde WebSocket interno al estar idle
- `createBrowserContext` → contexto aislado sin cookies = sin sesión
- Al volver a ChatGPT después de navegar X → "Something went wrong"

### Root cause análisis
1. **CDP Target.createTarget**: crea un target pero la navegación no es síncrona.
   `Page.navigate` retorna inmediatamente sin garantía de que Chrome procesará
   la navegación. En headless, el nuevo target puede estar en un estado "pending"
   donde los comandos CDP se ejecutan pero la navegación no se inicia.

2. **Cookies en tabs nuevas**: Las cookies del profile están en SQLite (Cookies DB).
   Chrome las carga al iniciar pero las tabs nuevas creadas via CDP pueden no
   tener acceso inmediato al cookie store hasta la primera navegación real.

3. **ChatGPT WebSocket**: ChatGPT mantiene un WebSocket interno para streaming.
   Cuando Chrome congela una tab (background/idle), este WS muere. Al reactivar,
   la página intenta reconectar pero a veces falla silenciosamente.

## Opciones

### Opción A: Single tab (simple, fiable)
Una sola tab. Navegar a ChatGPT, hablar, navegar a X, volver.

**Pros:**
- Funciona siempre — ya probado
- Sin complejidad de CDP tab management
- Cookies siempre disponibles

**Contras:**
- Pierdes el estado de ChatGPT al navegar a X
- Cada cambio de contexto = reload completo
- No puedes tener ChatGPT y X "abiertos" simultáneamente

### Opción B: Multi-tab con wait robusto
Crear tabs via `Target.createTarget` pero con un wait loop que garantice navegación.

**Pros:**
- ChatGPT persiste mientras navegas X
- Cada servicio tiene su propia tab

**Contras:**
- `Target.createTarget` + navegación no es fiable en headless Chrome
- Requiere keepalive thread para evitar que Chrome congele tabs idle
- Más código, más modos de fallo

### Opción C: Multi-window (no headless)
Lanzar Chrome en modo headed (sin `--headless=new`) con `--window-position=-10000,-10000` (fuera de pantalla).

**Pros:**
- Tabs funcionan como en un browser real
- `Target.createTarget` fiable
- No hay congelación de tabs idle
- Más compatible con sites anti-bot

**Contras:**
- Consume más memoria
- Requiere display (no funciona en servidor sin X11)
- Ventana invisible pero existe

### Opción D: Hybrid — single tab + ChatGPT API
Tab default para browsing. ChatGPT via API directa (no browser).

**Pros:**
- Browser solo para lo que necesita browser (X, LinkedIn, webs)
- ChatGPT via API es 100% fiable, sin DOM scraping
- Grok via API si disponible

**Contras:**
- Requiere API key de OpenAI (coste $)
- Pierde el "gratis" de usar la sesión web

## Arquitectura propuesta

### Modelo mental
```
Ghost Chrome = un browser real del usuario, pero invisible.
Cada tab = un sitio abierto, con su sesión.
Las cookies son por dominio, no por tab.
La navegación dentro de una tab cambia la URL pero mantiene la sesión.
```

### Session Manager
```python
class SessionManager:
    """Gestiona tabs y sesiones. Garantiza que cada servicio tiene su tab lista."""

    def __init__(self, chrome):
        self.chrome = chrome
        self.tabs = {}  # name → {ws, url, state, last_active}

    def get(self, name, url=None):
        """Obtener tab por nombre. Crea si no existe. Verifica estado."""
        tab = self.tabs.get(name)

        if tab:
            # Verificar que la tab está viva
            if self._is_alive(tab):
                tab['last_active'] = time.time()
                self.chrome._active = name
                return self.chrome
            else:
                # Tab muerta, recrear
                self._destroy(name)

        if not url:
            return None

        # Crear tab nueva con navegación garantizada
        self._create(name, url)
        return self.chrome

    def _create(self, name, url):
        """Crear tab con verificación completa."""
        # 1. Crear target
        # 2. Conectar WS
        # 3. Inyectar scripts
        # 4. Navegar
        # 5. ESPERAR: URL cambia + readyState=complete + DOM tiene contenido
        # 6. VERIFICAR: la página cargó correctamente (no error, no blank)
        pass

    def _is_alive(self, tab):
        """Verificar que la tab responde y tiene contenido."""
        try:
            state = self.chrome.js('return document.readyState')
            url = self.chrome.js('return location.href')
            return state == 'complete' and url != 'about:blank'
        except:
            return False

    def _destroy(self, name):
        """Cerrar tab y limpiar."""
        tab = self.tabs.pop(name, None)
        if tab:
            try: tab['ws'].close()
            except: pass
```

### Tab States
```
CREATING → NAVIGATING → LOADING → READY → ACTIVE → IDLE → (DEAD)
                                    ↑                        |
                                    └────── RECOVERING ←─────┘
```

### Cookie Strategy
```
1. Al arrancar Chrome: sync Cookies DB del Chrome real (ya funciona)
2. No crear BrowserContext aislados (rompe las cookies)
3. Todas las tabs comparten el mismo default context
4. Google cookies excluidas del sync (ya funciona)
5. Si una web devuelve login wall → re-sync cookies + retry (ya funciona)
```

### Decisión recomendada

**Opción B con estas garantías:**

1. **Tab creation**: `Target.createTarget` + esperar hasta que `location.href != about:blank`
2. **Navigation verification**: no confiar en `Page.navigate` return — poll hasta readyState=complete + hay DOM content
3. **Error recovery**: si la tab tiene "Something went wrong" → navegar a URL limpia
4. **Keepalive**: thread que hace `js('1')` cada 15s en tabs de chat
5. **State tracking**: dict con url, state, last_active por tab
6. **Fallback**: si después de 3 intentos la tab no funciona → usar tab default

## Preguntas para GPT

1. ¿Por qué `Target.createTarget` + `Page.navigate` falla en headless Chrome?
   ¿Es un bug conocido? ¿Hay un workaround?

2. ¿Cuál es la forma correcta de esperar a que una navegación CDP complete?
   `Page.loadEventFired`? `Page.frameStoppedLoading`?

3. ¿Cómo evitar que Chrome congele tabs idle en headless?
   ¿Hay un flag de Chrome o hay que hacer keepalive manual?

4. ¿Es mejor crear tabs con `Target.createTarget(url=target_url)` directamente
   en vez de crear en about:blank y luego navegar?

5. ¿Debería usar CDP sessions (`Target.attachToTarget`) en vez de
   conexiones WS separadas por tab?
