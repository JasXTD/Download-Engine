# Download Engine

## Kom i gang

### Se etter oppdateringer ved oppstart

Aktivert som standard og sjekker én gang hver gang appen åpnes. En ny versjon gir valget mellom oppdatering og senere. Nettverksfeil åpner ingen dialog. Kan slås av i Innstillinger. Last ned og installer oppdateringer fra den offisielle utgivelsessiden på GitHub.

Innstillinger åpnes i et vindu med kategorier. Lagre bruker endringene; Avbryt, × eller Escape forkaster ulagrede endringer. Temaer kan forhåndsvises; Tilbakestill gjelder bare gjeldende kategori. Avanserte redigerere lagrer separat og går tilbake til Innstillinger når de lukkes.

### Kom i gang

Velg Legg til URL, lim inn en direkte fillenke, kontroller navn, kø og mappe og start. Legg til gruppe tar én lenke per linje. Fra et mønster lager nummererte lenker som img001…img100 fra én lenke med *. Bytt språk i Innstillinger → Utseende → Språk. Systemspråk følger Windows; språk som ikke støttes bruker engelsk.



<!-- custom-themes-3.4.0 -->
### Temasamling

Innstillinger → Utseende → Eget tilbyr Neon Courier, Reactor Night, Aurora og Moonveil. Juster bakgrunn og aksent eller importer PNG/JPEG. Lagre beholder forhåndsvisningen; Avbryt gjenoppretter forrige tema. Bilder forblir lokale med størrelsesgrenser og bakgrunnsbehandling. Beskyttelse av profilversjoner og MCP-tilgangskontroller er også inkludert.

PNG / JPEG · opptil 16 MiB og 16 megapiksler. Kopieres til profilen.
<!-- /custom-themes-3.4.0 -->

## Nedlastinger og gjenoppretting

### Nedlastinger og gjenoppretting

Dobbeltklikk en nedlasting for fremdrift, hastighet og gjenstående tid. Pause og Stopp beholder lagrede data. Start umiddelbart prioriterer valgte filer. Fortsettelse krever serverstøtte og samsvarende data. Oppdater utløpte lenker i hurtigmenyen. Fjern fullførte fjerner bare historikk, ikke lagrede filer. Oppgaver → Eksporter liste… lagrer nedlastingene i en fil som sikkerhetskopi eller for en annen PC. Oppgaver → Importer liste… viser hva som er nytt før det legges til; eksisterende lenker hoppes over, og nye nedlastinger forblir pauset. Vis → Flytende slippboks viser en liten boks som ligger over andre vinduer; slipp lenker på den for å laste dem ned, også når Download Engine er i systemstatusfeltet.

## Frakoblede nettsteder

### Frakoblede nettsteder

Innsamling finner fillenker eller lagrer et nettsted frakoblet. Velg URL, dybde, side- og størrelsesgrense, og gjennomgå resultatene før du legger dem til. Åpne lagret index.html for å lese kopien. Dynamiske funksjoner, innlogging, skjemaer og krypterte videoer støttes ikke. Manglende ressurser står i rapporten.

## Nettleserintegrasjon

### Nettleserintegrasjon

Åpne Verktøy → Nettleseroppsett for Chrome, Edge, Brave eller Firefox. Chrome, Edge og Brave trenger den installerte utvidelsens faktiske ID. Velg Kontroller tilkobling i utvidelsen. Nettleservalg har eget språkvalg. Videoknappen åpner kvalitetsvalget. Nettstedsinnlogging brukes bare for den godkjente filen.

## Køer og tidsplaner

### Køer og tidsplaner

Administrer køer grupperer filer og begrenser samtidige nedlastinger. Nye køer venter på start eller tidsplan. Planleggeren har start, pause og gjentakelse. Appen må være åpen, også i systemstatusfeltet. Automatisk avslutning, avslåing, hvilemodus og dvalemodus aktiveres separat; nedtellingen kan avbrytes.

## Skript

Redigereren og innlastede skript har nå egne faner. Søk etter eller opprett skript i listen, og åpne den fullstendige redigereren fra overskriften for Lua eller PowerShell. Lagre, kjør/stopp og valgfrie logger vises sammen med koden. Filkommandoer ligger i redigererens handlingsmeny. Bytte av visning bevarer ulagret kode, innstillinger og markørposisjon.


### Skript

I Verktøy → Skript lager du regler etter nettsted eller filtype. Velg Lua eller PowerShell under Skript, lagre og kjør. Utløsere: manuelt, appstart, intervall, fullført nedlasting eller kø. Et skript overlapper ikke egne kjøringer. Stopp avslutter skriptet, men angrer ikke sendte nedlastingskommandoer. Utdata vises i kjøringsloggen.

## Lua-skript

### Lua-skript

Lua er innebygd. Opprett et manuelt Lua-skript, lagre eksemplet og kjør. xd.queues() gir køer og xd.downloads() nedlastinger. Fullførte nedlastinger gir event.downloads og køer event.queue. Ikke oversett kommandoer og feltnavn. Fullstendig API: docs/automation.md. Ikke logg hemmeligheter. PowerShell kjører med Windows-kontoens rettigheter.

Lua- og PowerShell-redigererne kontrollerer syntaks automatisk etter en kort skrivepause uten å lagre eller kjøre skriptet. Linjenumre, syntaksfarger og klikkbare feil er lagt til. Skriv starten av et navn eller trykk Ctrl+Space for forslag: velg med piltastene, sett inn med Tab, Enter eller klikk, og lukk med Esc. Fullføringer kan angres; kjøretidsfeil vises fortsatt i loggen.

```lua
print("Hello from Lua!")
print("Trigger:", event.trigger)
for _, queue in ipairs(xd.queues()) do
    print(queue.name, "running:", queue.running)
end
for _, file in ipairs(event.downloads) do
    print("Finished:", file.name or file.id)
end
```

Utløseren Nedlasting mislyktes kjører et skript når en nedlasting fortsatt trenger oppfølging etter Download Engines automatiske nye forsøk; event.downloads inneholder da error for hver nedlasting. xd.retry(id) og xd.restart(id) prøver igjen (skript kan gjøre det 5 ganger per nedlasting), xd.notify(title, text) viser et Windows-varsel, og xd.store.get og xd.store.set tar vare på verdier mellom kjøringer. xd.rename, xd.move og xd.remove endrer bare filen når Kan endre nedlastede filer er krysset av. Kjør med eksempel… prøver et hendelsesskript med nedlastinger du velger; endringer blir bare simulert.

```lua
for _, file in ipairs(event.downloads) do
    if pcall(xd.retry, file.id) then
        print("Retrying:", file.name or file.id)
    else
        xd.notify("Download still failing", (file.name or file.id) .. ": " .. (file.error or "no details"))
    end
end
local handled = (xd.store.get("handled") or 0) + #event.downloads
xd.store.set("handled", handled)
print("Failed downloads handled:", handled)
```

For å redigere et skript i et annet program bruker du Åpne fil… og Lagre til fil… i redigeringsprogrammets overskrift. Åpne fil… kopierer en .lua- eller .ps1-fil inn i redigeringsprogrammet og spør først hvis den ville erstatte kode som ikke er lagret; velg Lagre skript for å beholde den. Lagre til fil… skriver koden i redigeringsprogrammet til en fil.

### Lua Core API 1

Opprett eller åpne et skript i Loaded scripts; velg Lua og Manual only i Script settings. Full editor gir mer kodeplass. Velg Save script, Run script og les Run logs. Core API viser funksjoner for gjeldende utløser; Ctrl+Space åpner forslag. API 1 er kontraktversjonen, ikke appversjonen. Eksemplet tolker bare HTML i minnet, uten nettverk eller nedlastinger; sidehenting er false for et manuelt skript.

```lua
-- Example: core-api
assert(xd.api_version == 1, "This example needs Lua API 1")
print("App:", xd.app_version, "Role:", xd.script_kind)
local html = '<a class="download" href="/files/demo.zip">Demo</a>'
local links = xd.html_select(html, "a.download[href]", "href")
local url = xd.url_join("https://example.com/catalog/", links[1])
print("Preview:", xd.to_json({ kind = "file", url = url }))
print("Can fetch a page:", xd.supports("xd.fetch"))
```

Open examples/core-api-automation.lua or examples/custom-site-reader.lua with Editor actions → Open file…. Site reader uses public pages, Pages patterns and Direct connection; test with Test on URL before enabling its trigger. Ordinary automation cannot call xd.fetch. Import script… accepts .de-script.json as a new draft with automatic triggers and file changes off; review its code and reselect local queues. Stop script does not undo submitted downloads. Full English reference: docs/automation.md.


### Lua Provider API 1

Lua-leverandører finner kilder, behandler byte og bekrefter hasher med vanlige kontroller for pause og fortsettelse. Hver nedlasting beholder sin lagrede skriptversjon. Testene bruker bare lokalt genererte krypterte filer. Etter at en leverandør er lagret, kan eldre appversjoner ikke skrive til profilen.

Scripting → Script settings → Download provider → Save script → Add provider download.

API: `resolve(request)`, `refresh(request, previous)`, `transform(plan, bytes, offset)`, `finish(plan, result)`.

`docs/lua-providers.md` · `examples/download-provider.lua` · `examples/download-provider-custom-ctr.lua`

## Kommandolinje

### Kommandolinje

Åpne PowerShell i appmappen og bruk kommandoene nedenfor. Appen må kjøre for å motta styringskommandoer. JSON og kommandonavn forblir engelske. Sett stier med mellomrom i anførselstegn.

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

## MCP-integrasjon

### MCP-integrasjon

Start den lokale HTTP-serveren på MCP-siden og kopier klientkonfigurasjonen. Begynn med Bare visning. KI-klienten trenger URL og privat autentiseringsheader. Ikke publiser tokenet. Stopp serveren eller bytt token for å trekke tilbake tilgang. Detaljer: docs/mcp-ai-guide.md.

MCP åpnes nå i et popupvindu fra verktøylinjen, menyen Verktøy eller Innstillinger uten å bytte side. Lukk med × eller Escape; serveren fortsetter å kjøre og ulagrede MCP-endringer beholdes. Stopp serveren for å koble fra AI-klienter. Hvis vinduet ble åpnet fra Innstillinger, kommer du tilbake til samme utkast.

## Virusskanning

Skanner: Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV.

Krever antivirusprogrammet engelske Explorer-integrasjon. Tidsgrensen gjelder overleveringen, ikke skanningen. Avbryt skanninger i antivirusprogrammet.

Standard installasjonsmappe kreves.

Antivirusprogrammet styr beskyttelse og karantene. En forespørsel betyr ikke at filen er trygg.

Kontroller oppsett: Integrasjonen ble funnet; tilgjengelighet sjekkes når skanning forespørres.

### Virusskanning

Åpne Verktøy → Virusskanning. Enkeltfil velger én fullført nedlasting, Gruppe flere filer. En enkelt skanning kan avbrytes. Fanene er låst under skanning eller gjennomgang i Malwarebytes / Kaspersky / Bitdefender.

Velg antivirus i Innstillinger og åpne Verktøy → Virusskanning → Gruppe. Velg fullførte filer, start og gjennomgå resultatene. Stopp gruppe avbryter usendte forespørsler. Malwarebytes / Kaspersky / Bitdefender venter på din gjennomgang etter hver overlevering. Stopp overleverte skanninger i Malwarebytes / Kaspersky / Bitdefender. En sendt forespørsel betyr ikke at filen er trygg.

## Innstillinger og logger

### Innstillinger og logger

Innstillinger inneholder språk, tema, mapper, hastighet, datakvoter, proxy og tilkoblinger. Logger kan søkes, filtreres, kopieres og eksporteres. URL-er, stier, skriptutdata og tekniske feil beholder sin opprinnelige tekst. Gjennomgå rapporter før deling.

Den installerte versjonen leser Windows sin gjeldende nedlastingsmappe ved hver oppstart, også når den er flyttet til en annen disk. Nye nedlastinger bruker den plasseringen. Eksisterende nedlastinger, lagret fremdrift og egendefinerte mapper endres ikke. Den portable versjonen bruker appens plassering ved hver oppstart. Eldre profiler beholder undermappen IDM EX. En feil vises hvis plasseringen ikke kan hentes.

## Feilsøking

### Feilsøking

Ved tilkoblingsproblemer, kontroller at appen kjører, utvidelses-ID og nettleserregistrering. Last inn utvidelsen og siden på nytt etter oppdatering. Ved nedlastingsfeil, kontroller diskplass, mapperettigheter, proxy og lenkens gyldighet. Ikke slett gjenopprettingsdata før du har forsøkt å fortsette. Hvis tegn mangler, legg til Windows-språkskrifter og start på nytt.
