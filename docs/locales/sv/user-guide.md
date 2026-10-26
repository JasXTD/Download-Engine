# Download Engine

## Kom igång

Inställningar öppnas i en ruta med kategorier. Spara tillämpar ändringar; Avbryt, × eller Escape kastar osparade ändringar. Teman kan förhandsvisas; Återställ gäller endast aktuell kategori. Avancerade redigerare sparar separat och återgår till Inställningar när de stängs.

### Kom igång

Välj Lägg till URL, klistra in en direkt fillänk, granska namn, kö och mapp och starta. Lägg till batch tar en länk per rad. Från ett mönster skapar numrerade länkar som img001…img100 från en länk med *. Byt språk i Inställningar → Utseende → Språk. Systemspråk följer Windows; språk som inte stöds använder engelska.

## Hämtningar och återställning

### Hämtningar och återställning

Dubbelklicka på en hämtning för förlopp, hastighet och återstående tid. Paus och Stopp behåller sparade data. Starta direkt prioriterar valda filer. Återupptagning kräver serverstöd och matchande data. Uppdatera utgångna länkar i snabbmenyn. Rensa slutförda tar bara bort historik, inte sparade filer. Uppgifter → Exportera lista… sparar dina hämtningar i en fil som säkerhetskopia eller för en annan dator. Uppgifter → Importera lista… visar vad som är nytt innan det läggs till; befintliga länkar hoppas över och nya hämtningar förblir pausade. Visa → Flytande släppruta visar en liten ruta som ligger ovanpå andra fönster; släpp länkar på den för att hämta dem, även när Download Engine ligger i meddelandefältet.

## Offlinewebbplatser

### Offlinewebbplatser

Insamling hittar fillänkar eller sparar en webbplats offline. Välj URL, djup, sid- och storleksgräns och granska resultatet före tillägg. Öppna sparad index.html för att läsa kopian. Dynamiska funktioner, inloggning, formulär och krypterade videor stöds inte. Saknade resurser anges i rapporten.

## Webbläsarintegration

### Webbläsarintegration

Öppna Verktyg → Webbläsarinställning för Chrome, Edge, Brave eller Firefox. Chrome, Edge och Brave behöver det installerade tilläggets riktiga ID. Välj Kontrollera anslutning i tillägget. Webbläsaralternativ har eget språkval. Videoknappen öppnar kvalitetsvalet. Webbplatsinloggning används bara för den godkända filen.

## Köer och scheman

### Köer och scheman

Hantera köer grupperar filer och begränsar samtidiga hämtningar. Nya köer väntar på start eller schema. Schemaläggaren har start, paus och upprepning. Appen måste vara öppen, även i meddelandefältet. Automatisk avslutning, avstängning, strömsparläge och viloläge aktiveras separat; nedräkningen kan avbrytas.

## Skript

### Skript

I Verktyg → Skript skapar du regler efter webbplats eller filändelse. Välj Lua eller PowerShell under Skript, spara och kör. Utlösare: manuellt, appstart, intervall, slutförd hämtning eller kö. Ett skript överlappar inte sina egna körningar. Stopp avslutar skriptet men ångrar inte skickade hämtningskommandon. Utdata visas i körloggen.

## Lua-skript

### Lua-skript

Lua är inbyggt. Skapa ett manuellt Lua-skript, spara exemplet och kör. xd.queues() ger köer och xd.downloads() hämtningar. Slutförda hämtningar ger event.downloads och köer event.queue. Översätt inte kommandon och fältnamn. Fullständigt API: docs/automation.md. Logga inte hemligheter. PowerShell körs med ditt Windows-kontos rättigheter.

Lua- och PowerShell-redigerarna kontrollerar syntax automatiskt efter en kort skrivpaus utan att spara eller köra skriptet. Radnummer, syntaxfärger och klickbara fel har lagts till. Skriv början av ett namn eller tryck Ctrl+Space för förslag: välj med pilarna, infoga med Tab, Enter eller klick och stäng med Esc. Kompletteringar kan ångras; körningsfel finns fortfarande i loggen.

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

Utlösaren Hämtningen misslyckades kör ett skript när en hämtning fortfarande behöver åtgärdas efter Download Engines automatiska nya försök; event.downloads innehåller då varje hämtnings error. xd.retry(id) och xd.restart(id) försöker igen (skript får göra det 5 gånger per hämtning), xd.notify(title, text) visar en Windows-avisering och xd.store.get och xd.store.set sparar värden mellan körningar. xd.rename, xd.move och xd.remove ändrar bara filen när Får ändra hämtade filer är markerat. Kör med exempel… provar ett händelseskript med hämtningar du väljer; ändringar simuleras bara.

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

Om du vill redigera ett skript i ett annat program använder du Öppna fil… och Spara till fil… i redigerarens rubrik. Öppna fil… kopierar en .lua- eller .ps1-fil till redigeraren och frågar först om den skulle ersätta osparad kod; välj Spara skript för att behålla den. Spara till fil… skriver redigerarens kod till en fil.

## Kommandorad

### Kommandorad

Öppna PowerShell i appmappen och använd kommandona nedan. Appen måste köras för att ta emot styrkommandon. JSON och kommandonamn förblir engelska. Sätt sökvägar med mellanslag inom citattecken.

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

## MCP-integration

### MCP-integration

Starta den lokala HTTP-servern på MCP-sidan och kopiera klientkonfigurationen. Börja med Endast visning. AI-klienten behöver URL och privat autentiseringshuvud. Publicera inte token. Stoppa servern eller byt token för att återkalla åtkomst. Detaljer: docs/mcp-ai-guide.md.

MCP öppnas nu i ett popupfönster från verktygsfältet, menyn Verktyg eller Inställningar utan att byta sida. Stäng med × eller Escape; servern fortsätter att köras och osparade MCP-ändringar behålls. Stoppa servern för att koppla från AI-klienter. Om fönstret öppnades från Inställningar återgår du till samma utkast.

## Virussökning

Skanner: Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV.

Kräver antivirusprogrammet engelska Explorer-integration. Tidsgränsen gäller överlämningen, inte skanningen. Avbryt skanningar i antivirusprogrammet.

Standardmappen för installation krävs.

Antivirusprogrammet styr skydd och karantän. En begäran betyder inte att filen är säker.

Kontrollera inställningar: Integrationen hittades; tillgänglighet kontrolleras när en skanning begärs.

### Virussökning

Öppna Verktyg → Virussökning. Enstaka väljer en slutförd nedladdning, Batch flera filer. En enstaka skanning kan avbrytas. Flikarna är låsta under skanning eller granskning i Malwarebytes / Kaspersky / Bitdefender.

Välj antivirus i Inställningar och öppna Verktyg → Virussökning → Batch. Välj slutförda filer, starta och granska resultaten. Stoppa batch avbryter oskickade förfrågningar. Malwarebytes / Kaspersky / Bitdefender väntar på din granskning efter varje överlämning. Stoppa överlämnade sökningar i Malwarebytes / Kaspersky / Bitdefender. En skickad begäran betyder inte att filen är säker.

## Inställningar och loggar

### Inställningar och loggar

Inställningar innehåller språk, tema, mappar, hastighet, datakvoter, proxy och anslutningar. Loggar kan sökas, filtreras, kopieras och exporteras. URL:er, sökvägar, skriptutdata och tekniska fel behåller sin ursprungliga text. Granska rapporter innan du delar dem.

Den installerade versionen läser Windows aktuella nedladdningsmapp vid varje start, även om den flyttats till en annan enhet. Nya nedladdningar använder den platsen. Befintliga nedladdningar, sparade framsteg och egna mappar ändras inte. Den portabla versionen utgår från appens plats vid varje start. Äldre profiler behåller undermappen IDM EX. Ett fel visas om platsen inte kan hämtas.

## Felsökning

### Felsökning

Vid anslutningsproblem, kontrollera att appen körs samt tilläggs-ID och webbläsarregistrering. Ladda om tillägget och sidan efter uppdatering. Vid hämtningsfel, kontrollera diskutrymme, mappbehörighet, proxy och länkens giltighet. Radera inte återställningsdata innan du försökt återuppta. Om tecken saknas, lägg till Windows-språktypsnitt och starta om.
