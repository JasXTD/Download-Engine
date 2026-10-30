## 3.6.0 — 2026-10-31

### Tillägg

Den nya sidan Tillägg ersätter Skript och har flikarna Installerade, Skript, Regler och Körhistorik. Ett tillägg är en enda .de-extension.json-fil som samlar webbplatsläsare, nedladdningsleverantörer och automatiseringsjobb. Installation sker endast från en lokal fil, och utgivare verifieras inte. En granskning visar vad tillägget kan göra, det startar avstängt och en uppdatering måste fråga innan den får ny åtkomst. Webbåtkomsten är begränsad till de värdar som tillägget anger. Efter att det första tillägget installerats kan äldre appversioner inte skriva till profilen.

## 3.5.2 — 2026-10-30

### Sök efter uppdateringar vid start

Aktiverat som standard och kontrollerar en gång varje gång appen öppnas. En ny version ger valet att hämta uppdateringen eller vänta. Nätverksfel öppnar ingen dialog. Kan stängas av i Inställningar. Hämta och installera uppdateringar från den officiella releasesidan på GitHub.

## 3.5.1 — 2026-10-29

### Säkerhet för Lua-leverantörer

- Resolve/refresh och transform/finish körs i processer med separata behörigheter. Bearbetningsprocessen får aldrig `xd.fetch`, inte ens när källkoden laddas. Använd `xd.phase` för fasberoende initiering; resolverns lokala variabler förs inte över.
- Sparade planer innehåller bara storlek, hashvärden och transformations-ID. Upplösta URL:er förnyas vid behov; källkoden som låsts till jobbet skyddas av DPAPI för den aktuella Windows-användaren. Atomär uppgradering bevarar kontrollpunkternas bindningar och blockerar skrivning från äldre versioner.
- `xd.key` och `scripts protect-key`, som läser nycklar från omdirigerad stdin, ger ogenomskinliga AES-nyckelhandtag bundna till leverantören. Hemliga nycklar behöver inte finnas i kod eller exporter. Äldre säkerhetskopior och nycklar som redan skrivits i bibliotekets skript raderas inte; se `docs/lua-providers.md`.

### Rensa historik

Historiken för slutförda hämtningar kräver nu en bekräftelse som visar kön och antalet poster. Avbryt eller stäng för att behålla historiken. Hämtade filer sparas.

## 3.5.0 — 2026-10-29

### Lua Provider API 1

Lua-leverantörer löser källor, bearbetar byte och verifierar hashvärden med vanliga paus- och återupptagningskontroller. Varje nedladdning behåller sin sparade skriptversion. Testerna använder endast lokalt skapade krypterade filer. Efter att en leverantör sparats kan äldre appversioner inte skriva till profilen.

## 3.4.1 — 2026-10-29

Listan minskar upprepad kopiering och sökning i stora köer. Små fönster ger mer plats åt filnamn. Starta eller pausa kön bredvid dess status; starten åsidosätter schemat. Håll pekaren över statusen för information om återförsök och fel. Överföring, verifiering och formatet för återupptagning är oförändrade.

## 3.4.0 — 2026-10-29

### Temasamling

Inställningar → Utseende → Eget erbjuder Neon Courier, Reactor Night, Aurora och Moonveil. Justera bakgrund och accent eller importera PNG/JPEG. Spara behåller förhandsvisningen; Avbryt återställer tidigare tema. Bilder stannar lokalt med storleksgränser och bearbetning i bakgrunden.

### Förbättringar av profiler och kontroller

Även skydd för profilversioner och MCP-åtkomstkontroller ingår.
