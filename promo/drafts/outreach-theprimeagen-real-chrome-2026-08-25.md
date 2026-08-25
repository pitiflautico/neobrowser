# Outreach — ThePrimeagen

## Ángulo
Toma contraria sobre el spoofing de headless browsers, con tono técnico y algo de humor. No pedir promoción, compartir un experimento interesante.

## Vía
X reply a un post sobre AI agents, browser automation, o "things devs get wrong". Si X sigue bloqueado, borrador para publicación manual.

## Draft

```
hot take: every "stealth" headless browser for AI agents is a losing arms race.

you patch navigator.webdriver, they check webgl unmasked vendor. you spoof webgl, they check client hints vs ua. you patch those, they check perf.now() jitter. every spoof is a future flag.

we went the other way with neobrowser: don't spoof, just drive the user's real chrome. same cookies, same fingerprint, same gpu. passes sannysoft with the *genuine* values.

trade-off? not the fastest. but there are flows headless simply can't do.

repo + honest benchmark: github.com/pitiflautico/neobrowser
```

## Por qué funciona
- Encaja con su estilo de takes técnicos.
- No es una petición explícita.
- Link al benchmark, no al repo raíz.

## Estado
Borrador listo. X bloqueado por CAPTCHA; publicación manual pendiente.
