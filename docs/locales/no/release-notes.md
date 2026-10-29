## 3.5.2 — 2026-10-30

### Se etter oppdateringer ved oppstart

Aktivert som standard og sjekker én gang hver gang appen åpnes. En ny versjon gir valget mellom oppdatering og senere. Nettverksfeil åpner ingen dialog. Kan slås av i Innstillinger. Last ned og installer oppdateringer fra den offisielle utgivelsessiden på GitHub.

## 3.5.1 — 2026-10-29

### Sikkerhet for Lua-leverandører

- Resolve/refresh og transform/finish kjører i prosesser med separate rettigheter. Behandlingsprosessen får aldri `xd.fetch`, heller ikke under innlasting av kildekoden. Bruk `xd.phase` til faseavhengig initialisering; lokale variabler fra resolveren overføres ikke.
- Lagrede planer inneholder bare størrelse, hasher og transformasjons-ID. Avklarte URL-er fornyes ved behov; kildekoden som er låst til jobben, beskyttes av DPAPI for gjeldende Windows-bruker. Atomisk oppgradering bevarer kontrollpunktenes bindinger og blokkerer skriving fra eldre versjoner.
- `xd.key` og `scripts protect-key`, som leser nøkler fra omdirigert stdin, gir ugjennomsiktige AES-nøkkelhåndtak bundet til leverandøren. Hemmelige nøkler trenger ikke ligge i kode eller eksport. Eldre sikkerhetskopier og nøkler som allerede står i bibliotekets skript, slettes ikke; se `docs/lua-providers.md`.

### Tøm historikk

Tømming av historikken for fullførte nedlastinger krever nå bekreftelse med kø og antall oppføringer. Avbryt eller lukk for å beholde historikken. Nedlastede filer beholdes.

## 3.5.0 — 2026-10-29

### Lua Provider API 1

Lua-leverandører finner kilder, behandler byte og bekrefter hasher med vanlige kontroller for pause og fortsettelse. Hver nedlasting beholder sin lagrede skriptversjon. Testene bruker bare lokalt genererte krypterte filer. Etter at en leverandør er lagret, kan eldre appversjoner ikke skrive til profilen.

## 3.4.1 — 2026-10-29

Listen reduserer gjentatt kopiering og søking i store køer. Små vinduer gir mer plass til filnavn. Start eller sett køen på pause ved statusen; start overstyrer tidsplanen. Hold pekeren over statusen for forklaringer på nye forsøk og feil. Overføring, kontroll og formatet for gjenopptakelse er uendret.

## 3.4.0 — 2026-10-29

### Temasamling

Innstillinger → Utseende → Eget tilbyr Neon Courier, Reactor Night, Aurora og Moonveil. Juster bakgrunn og aksent eller importer PNG/JPEG. Lagre beholder forhåndsvisningen; Avbryt gjenoppretter forrige tema. Bilder forblir lokale med størrelsesgrenser og bakgrunnsbehandling.

### Forbedringer i profiler og kontroller

Beskyttelse av profilversjoner og MCP-tilgangskontroller er også inkludert.
