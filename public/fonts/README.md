# Bundled fonts

Vendored so the app renders identically offline — a desktop SDR tool cannot
depend on a font CDN being reachable, and fetching one leaks a request on
every launch.

| File | Family | Licence |
|---|---|---|
| `ibm-plex-mono-{400,500,600}.woff2` | IBM Plex Mono | SIL Open Font License 1.1 |
| `vt323-400.woff2` | VT323 | SIL Open Font License 1.1 |

Latin subset only (the UI ships no other script). Re-fetch from Google Fonts
if a new weight is needed; keep the subset narrow.
