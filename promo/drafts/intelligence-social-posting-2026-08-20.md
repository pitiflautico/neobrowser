# Inteligencia: social posting con neobrowser (2026-08-20)

## Estado de canales

### Reddit r/mcp
- **Sesión:** válida (posts anteriores publicados con éxito).
- **Text posts:** funcionan.
- **Media posts (GIF):** múltiples intentos no verificados. Posibles causas:
  1. Old Reddit requiere interacción específica con el file input nativo que `upload` no completa.
  2. Rate-limit o moderación automática por publicar demasiado seguido.
  3. El formulario de old.reddit.com/submit puede tener validación adicional que no se dispara con `submit`.
- **Lección:** para Reddit, priorizar posts de texto con link hasta resolver el upload de media.

### X / Twitter
- **Sesión:** válida en el perfil de neobrowser (Daniel Perez Pinazo / @perez_pin).
- **Compose:** el editor es `contenteditable` con `data-testid="tweetTextarea_0"`; hay 2 file inputs.
- **Post con GIF:** intentado. El compose quedó vacío tras el envío, lo que sugiere que el post pudo haberse publicado. Verificación inconclusa porque el perfil/timeline no cargan tweets recientes (posible restricción de X o carga lazy).
- **Lección:** X es funcional con neobrowser; se necesita una estrategia de verificación más robusta (URL del tweet o notificaciones).

## Comparativa contenido
- **Texto + link:** más fácil de automatizar, menor engagement visual.
- **GIF demo:** mucho mayor potencial de engagement, pero la subida de media es frágil en old Reddit.

## Táctica recomendada
1. **X:** usar como canal principal para GIFs cortos. Publicar en horario US (9–11am ET) para máximo alcance.
2. **Reddit:** usar posts de texto con el GIF como link externo (o imgur/GitHub raw) para evitar el upload nativo.
3. **LinkedIn:** requiere sesión estable; pedir al usuario que inicie sesión en el perfil de neobrowser.
4. **Verificación:** tras cada post, capturar URL o screenshot de confirmación; si no es posible, marcar como "inconcluso" y no forzar reintentos.

## Métricas a trackear
- Vistas del repo en las próximas 24h (GitHub Insights).
- Estrellas ganadas por canal (imposible de atribuir 100%, pero se puede correlacionar timing).
- Engagement del post (likes, replies, retweets).
