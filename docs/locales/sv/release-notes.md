## 3.19.1 — 2026-11-10

### Uppdaterat Linux-paket för Wayland

- Linux-paketet aktiverar stöd för överordnade fönster i systemdialoger. Filval, spara, avbryt och överskrivning har testats. Versionen är fortfarande 3.19.1; ladda ned paketet igen manuellt. Windows- och macOS-paketen är oförändrade.
- Rapportera fel via [GitHub Issues](https://github.com/JasXTD/Download-Engine/issues) i stället för Reddit så att rapporter och korrigeringar kan följas upp.

### Rättad Dailymotion-identifiering och videoförhandsvisning

- Rättat Dailymotion-identifieringen och nedladdningsknappen. Om en länk inte fungerar provar appen en alternativ spellista för samma video. Förbättrade diagnostikloggar utan att spara känsliga uppgifter.

[English details](../../../CHANGELOG.md#3191--2026-11-10)

## 3.19.0 — 2026-11-08

### BitTorrent v2 och hybridtorrenter

- Stöd för v2- och hybridtorrenter med SHA-256-kontroll, filval, återupptagning och återställning. V1 och befintliga delningsinställningar behålls.

[English details](../../../CHANGELOG.md#3190--2026-11-08)

## 3.18.1 — 2026-11-08

- Uppladdning är avstängd som standard. Du kan aktivera den i Inställningar → Torrenter.
- **Tillåt inkommande anslutningar?** Andra klienter kan ansluta till appen. Det ökar exponeringen mot nätverket. Brandväggens och routerns inställningar ändras inte.
- **Aktivera UPnP?** Routern ombeds vidarebefordra torrentens TCP-port medan torrenter körs. Andra klienter kan då eventuellt nå appen från internet. Spara inställningarna för att tillämpa ändringen.

[English details](../../../CHANGELOG.md#3181--2026-11-08)

## 3.18.0 — 2026-11-08

### VPN / nätverksgränssnitt

- Bind torrenter till en gränssnittsadress. Överföringar stannar om den försvinner; andra nedladdningar behåller sina inställningar.
- DNS använder också detta gränssnitt. Använd samma IP-version som den valda adressen. Routerns portmappning är avstängd under bindningen.
- TCP / uTP · Krypterad / Klartext

[English details](../../../CHANGELOG.md#3180--2026-11-08)

## 3.17.0 — 2026-11-08

### Kryptering mellan peers, uTP och webbkällor

Lagt till valfri MSE/PE-kryptering, uTP-anslutningar och HTTP/HTTPS-webbkällor. Alla tre är avstängda som standard och kan aktiveras i torrentinställningarna. Delar verifieras och källor sparas för återupptagning efter omstart. Paus, återupptagning och stoppa alla har förbättrats. Kryptering döljer inte din IP-adress.

[English details](../../../CHANGELOG.md#3170--2026-11-08)

## 3.16.1 — 2026-11-08

### Rättningar av inställningar och kontroller

Tilläggsinställningar sparar nu tom text och hela formuläret i en enda transaktion. Osparade ändringar behålls när du byter tillägg. Spara-knappen är synlig i små fönster och uppdateringar läser in standardvärden igen. Stoppmenyer för delade torrenter och visningen av hastighetsgränser har rättats; upprepade förloppsskrivningar har minskats. Rapportexportens text och informationen om systemskydd och uppdateringsinställningar har korrigerats.

[English details](../../../CHANGELOG.md#3161--2026-11-08)

## 3.16.0 — 2026-11-07

### Lua-exempel och behörighetskontroll

Fem Lua-tillägg ingår: offentliga Internet Archive-filer, Wikimedia Commons-original, GitHub-versionsfiler, sortering av färdiga filer efter typ samt aviseringar när kön är klar med valfri start av nästa kö. Välj dem under Extensions → Examples och granska behörigheterna före installation; medföljande tillägg körs inte automatiskt. Den tomma tilläggssidan, installationsgranskningen och behörigheter per jobb har förbättrats. Filåtkomst kan beviljas eller återkallas efter installation, och en guide beskriver det första egna tillägget.

Ändrade paket kräver en ny granskning. Reserverade exempel-ID:n kräver exakt de medföljande filerna, och vilseledande kontrolltecken avvisas i metadata. Tillägg hittas fortfarande i appens plugins-mapp. Saknade översättningar av kraschrapporter och gränssnitt har lagts till, upptäckten av översättningsbara texter har förbättrats och rätoromanskan har gjorts enhetlig. Saknade äldre versionsanteckningar har återställts; macOS/Linux-paket innehåller länkade guider och bilder för dokumentation offline.

[English details](../../../CHANGELOG.md#3160--2026-11-07)

## 3.15.0 — 2026-11-06

### Lua-leverantörer och ASS-undertexter

HTTP i Lua stöder nu HEAD, PUT, PATCH, DELETE och OPTIONS, binära meddelandekroppar med storleksgränser och uttryckligen angivna sessioner bundna till ett ursprung. Inloggningsuppgifter förblir krypterade utanför adresslösaren och sparas inte i planer eller skickas till offlineprocessorer. Medieplaner tillåter segment med okänd storlek; lua_stream behandlar begränsade datablock med verifierade kontrollpunkter för paus, återupptagning, stopp och återställning.

Tillagt: import av binära/RSA-inloggningsuppgifter skyddade av operativsystemet, slumpbyte, SHA-1/SHA-256, HMAC, RSA-signering och dekryptering samt dekryptering av AES-block. ASS-undertexter behåller stilar och inbäddade formateringsanvisningar i MKV, medan MP4 omvandlar dem till textundertexter. Nya funktioner för inloggningsuppgifter och medier kräver en app som stöder profilens skrivversion 9. Befintliga kontrollpunktsformat är fortsatt kompatibla.

CENC-labbet för författare körs offline och använder Lua för att tolka en begränsad del av AVC/AAC-sampelkryptering, simulera ett signerat licensutbyte, överlämna nycklar och sammanfoga dekrypterade spår med ASS-undertexter. Webbplatsautentisering, licensformat och containertolkning ligger fortsatt i Lua-koden. Testerna använder genererade data utan verkliga tjänster eller konton; detta är inte en fullständig CENC-implementation.

### Delning och sökning av torrenter

Delning av en färdig torrent stoppar nu av sig själv vid kvoten 2.0 eller efter en vecka, beroende på vad som inträffar först. Det är på som standard, även för befintliga profiler; båda gränserna kan ändras eller stängas av under Inställningar → Torrenter. Uppladdning under hämtning räknas in i kvoten. Statusfönstret visar kvoten, och en torrent som stoppats av en gräns behåller sina filer och erbjuder Dela igen i sin meny.

Magnetlänkar utan tracker avvisas inte längre direkt. Fönstret Lägg till hämtning förklarar att andra datorer vid en DHT-sökning ser vilken torrent du vill ha och din IP-adress, och söker först när du klickar på Sök i DHT. Din dator annonseras som peer först när den kontrollerade fillistan visar en offentlig torrent. En privat torrent stoppar sökningen, och en sökning avslutas efter 10 minuter. Länkar från webbläsare, urklipp eller Utforskaren startar aldrig en sökning av sig själva.

Nytt: Konfigurera routern (UPnP), avstängt som standard precis som inkommande anslutningar, som det kräver. Det kommunicerar bara med standardgatewayen i det privata nätverket, begär en enda vidarebefordringsregel som gäller en timme i taget och tar bara bort sin egen regel; macOS rapporterar det som otillgängligt. Upptäckta peers och DHT-noder på andra tjänsters portar (under 1024 samt portar som SIP 5060) kontaktas aldrig. Hämtningar som stoppas av en DHT-orsak visar nu orsaken på det valda språket.

[English details](../../../CHANGELOG.md#3150--2026-11-06)

## 3.10.2 — 2026-11-04

### Lokala kraschrapporter

Hjälp och Loggar låter dig granska, kopiera och exportera rapporter om Rust-panik och onormal avslutning. Ingen automatisk uppladdning; lösenord, nedladdningsadresser, personliga sökvägar och minnesdumpar ingår inte.

[English details](../../../CHANGELOG.md#3102--2026-11-04)

## 3.10.1 — 2026-11-04

### Säkerhet och tillförlitlighet

Förbättrade resursgränser för sökning efter torrentpeers, avbrytning och filtrering av nätverksadresser. Lagringen av inloggningsuppgifter i macOS och Linux har uppdaterats med bevarade inloggningar. Befintliga mål skrivs inte över när torrentmappar sparas.

[English details](../../../CHANGELOG.md#3101--2026-11-04)

## 3.10.0 — 2026-11-03

### macOS och Linux

Nativa paket för Mac med Apple Silicon och Intel samt ett Linux x64-arkiv använder samma gränssnitt och nedladdningsmotor som Windows. FFmpeg med källkod och licenser ingår, tillsammans med systemets fildialoger och lagring av inloggningsuppgifter. Mac-paketen är ad hoc-signerade och inte notariserade av Apple. Se guiden för krav och testbegränsningar.

[Guide / English](../../cross-platform.md)

## 3.9.0 — 2026-11-01

### Torrenthantering

Stöd för magnetlänkar från webbläsaren, släppta torrentfiler och förslag från urklipp. Delning efter slutförande och inkommande anslutningar är separata alternativ som är avstängda som standard. Flera filer kan skannas och mappar flyttas eller byta namn; Stoppa alla stoppar även delningen.

[Guide / English](../../user-guide.md#download-torrents)

## 3.8.0 — 2026-11-01

### Öppna torrenter från Windows

Öppna .torrent-filer från Utforskaren och magnetlänkar från webbläsaren för granskning. Valfri Windows-registrering behåller standardappar, och väntande granskningar finns kvar efter omstart. Torrentskyddet har stärkts: magneter utan cachade metadata kräver en tracker, annars använder du en .torrent-fil. Regressionstester täcker det befintliga skyddet mot filnamnskonflikter.

## 3.7.0 — 2026-11-01

### Torrentnedladdningar

Torrentfiler och magnetlänkar med filval, paus, återupptagning och återställning av verifierade data. Avstängt som standard; aktivera och styr uppladdning i inställningarna. En aktiv proxy pausar torrenter och uppladdningen slutar när nedladdningen är klar. Innehåller rättningarna från 3.6.1 och torrenthjälpprogrammet.

### Versionsintegration

Guiden, README, webbplatsen och installationskontrollerna har uppdaterats. När den första torrenten lagts till kräver profilen version 3.7.0 eller senare så att äldre versioner inte kan ändra torrentjobb. Att enbart aktivera funktionen uppgraderar inte profilen.

## 3.6.1 — 2026-10-31

### Färre beroenden och godkännande av tillägg

Rusts inbyggda fillås ersätter fs2 och tar bort det äldre winapi-beroendet, med samma lås- och väntbeteende. Tilläggsuppdateringar som lägger till jobb, även genom att ersätta ett jobb-ID, kräver nu godkännande även när tillägget är avstängt.

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

## 3.3.3 — 2026-10-28

### Förbättrat gränssnitt

Redigeraren och inlästa skript har nu separata flikar. Sök eller skapa skript i listan och öppna den fullständiga redigeraren från rubriken för Lua eller PowerShell. Spara, kör/stoppa och valfria loggar finns nära koden. Filkommandon ligger i redigerarens åtgärdsmeny. Byte av vy bevarar osparad kod, inställningar och markörens position.

### Snabbare respons vid nedladdningar

Uppdatering av köer och status, videoförfrågningar från webbläsaren och nya nedladdningar hanteras i bakgrunden så att långsam lagring inte blockerar klick. Dialogerna visar förloppet, förhindrar dubbla inlämningar och behåller utkastet om det inte går att spara. Mappar öppnas också i bakgrunden, och äldre uppdateringar ersätter inte den nyvalda kön.
