# PDR: NeoBrowser Plugins

## Qué es un Plugin

Un plugin es un pipeline reutilizable de acciones del browser.
Se define en YAML, se guarda en `~/.neorender/plugins/`, y se ejecuta como un tool más.

## Ejemplo: Twitter Monitor

```yaml
# ~/.neorender/plugins/twitter-monitor.yaml
name: twitter-monitor
description: Lee los últimos tweets de varias cuentas de X/Twitter
inputs:
  accounts:
    type: list
    description: Lista de cuentas a monitorear
    default: ["elonmusk", "sama", "kaborodev"]
  count:
    type: number
    description: Tweets por cuenta
    default: 5

steps:
  - action: open
    url: "https://x.com/{{account}}"
    wait: 8000
    loop: accounts  # repite para cada account
    as: account

  - action: scroll
    amount: 1000
    repeat: 3

  - action: extract
    type: tweets  # extractor custom
    save_as: "tweets_{{account}}"

output:
  format: markdown
  template: |
    # Twitter Monitor — {{timestamp}}
    {% for account in accounts %}
    ## @{{account}}
    {{tweets[account]}}
    {% endfor %}
```

## Ejemplo: Form Submitter

```yaml
name: contact-form
description: Rellena y envía un formulario de contacto
inputs:
  url:
    type: string
  name:
    type: string
  email:
    type: string
  message:
    type: string

steps:
  - action: open
    url: "{{url}}"
    wait: 5000

  - action: smart_fill
    fields:
      name: "{{name}}"
      email: "{{email}}"
      message: "{{message}}"

  - action: submit

  - action: wait
    text: "sent|thank|gracias|enviado"
    wait: 10000

  - action: read
    save_as: result

output:
  format: text
  template: "Form submitted: {{result}}"
```

## Ejemplo: Price Tracker

```yaml
name: price-tracker
description: Compara precios de un producto en varias tiendas
inputs:
  product:
    type: string
  stores:
    type: list
    default: ["amazon.es", "pccomponentes.com", "mediamarkt.es"]

steps:
  - action: search
    query: "{{product}} site:{{store}}"
    num: 1
    loop: stores
    as: store
    save_as: "search_{{store}}"

  - action: open
    url: "{{search[store].first_url}}"
    loop: stores
    as: store
    wait: 5000

  - action: extract
    type: price
    save_as: "price_{{store}}"

output:
  format: table
  columns: [store, price, url]
```

## Arquitectura

```
~/.neorender/plugins/
  twitter-monitor.yaml
  contact-form.yaml
  price-tracker.yaml
  my-custom-plugin.yaml

neo-browser MCP:
  tool: plugin
    action: run    → ejecuta un plugin
    action: list   → lista plugins disponibles
    action: create → crea plugin desde descripción
```

## Plugin Engine

1. Lee YAML del plugin
2. Parsea inputs + defaults
3. Ejecuta steps secuencialmente (o en loop)
4. Cada step llama a las tools existentes (open, click, type, fill, etc.)
5. Variables {{}} se resuelven con Jinja2-style templating
6. Output se formatea según template
7. Resultado se devuelve al LLM
