## 3.9.0 — 2026-11-01

### Torrentarbeidsflyt

Støtte for magnetlenker fra nettleseren, dra og slipp av torrentfiler og forslag fra utklippstavlen. Deling etter fullføring og innkommende tilkoblinger er separate valg som er av som standard. Flere filer kan skannes og mapper flyttes eller gis nytt navn; Stopp alle stopper også deling.

[Guide / English](../../user-guide.md#download-torrents)

## 3.8.0 — 2026-11-01

### Åpne torrenter fra Windows

Åpne .torrent-filer fra Utforsker og magnetlenker fra nettleseren for gjennomgang. Valgfri Windows-registrering beholder standardapper, og ventende gjennomganger bevares etter omstart. Torrentbeskyttelsen er styrket: magneter uten hurtigbufrede metadata krever en tracker, ellers bruker du en .torrent-fil. Regresjonstester dekker den eksisterende beskyttelsen mot filnavnkonflikter.

## 3.7.0 — 2026-11-01

### Torrentnedlastinger

Torrentfiler og magnetlenker med filvalg, pause, fortsettelse og gjenoppretting av bekreftede data. Av som standard; aktiver og styr opplasting i innstillingene. En aktiv proxy setter torrenter på pause, og opplasting stopper når nedlastingen er ferdig. Inkluderer rettelsene fra 3.6.1 og torrenthjelperen.

### Versjonsintegrering

Veiledningen, README, nettstedet og installasjonskontrollene er oppdatert. Etter at den første torrenten er lagt til, krever profilen versjon 3.7.0 eller nyere slik at eldre versjoner ikke endrer torrentjobber. Bare å aktivere funksjonen oppgraderer ikke profilen.

## 3.6.1 — 2026-10-31

### Færre avhengigheter og godkjenning av utvidelser

Rusts innebygde fillåser erstatter fs2 og fjerner den eldre winapi-avhengigheten, med samme låse- og venteoppførsel. Utvidelsesoppdateringer som legger til jobber, også ved å erstatte en jobb-ID, krever nå godkjenning selv når utvidelsen er slått av.

## 3.6.0 — 2026-10-31

### Utvidelser

Den nye siden Utvidelser erstatter Skript og har fanene Installert, Skript, Regler og Kjøringshistorikk. En utvidelse er én .de-extension.json-fil som samler nettstedslesere, nedlastingsleverandører og automatiseringsjobber. Installasjon skjer bare fra en lokal fil, og utgivere blir ikke verifisert. En gjennomgang viser hva utvidelsen kan gjøre, den starter avslått, og en oppdatering må spørre før den får ny tilgang. Nettilgangen er begrenset til vertene utvidelsen oppgir. Etter at den første utvidelsen er installert, kan eldre appversjoner ikke skrive til profilen.

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
