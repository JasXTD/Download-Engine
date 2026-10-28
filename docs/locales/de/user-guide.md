# Download Engine

## Erste Schritte

Einstellungen öffnen sich jetzt als Popup mit Kategorien. Speichern übernimmt Änderungen; Abbrechen, × oder Escape verwirft ungespeicherte Änderungen. Designs lassen sich vorab ansehen; Zurücksetzen betrifft nur die aktuelle Kategorie. Erweiterte Editoren speichern separat und kehren beim Schließen zu den Einstellungen zurück.

### Erste Schritte

URL hinzufügen wählen, einen direkten Dateilink einfügen, Name, Warteschlange und Ordner prüfen und starten. Stapel hinzufügen nimmt einen Link pro Zeile an. „Aus einem Muster“ erzeugt nummerierte Links wie img001…img100 aus einem Link mit *. Die Anzeigesprache ändern Sie unter Einstellungen → Darstellung → Sprache. Systemsprache folgt Windows; nicht unterstützte Sprachen verwenden Englisch.



<!-- custom-themes-3.4.0 -->
### Themensammlung

Einstellungen → Darstellung → Eigene bietet Neon Courier, Reactor Night, Aurora und Moonveil. Hintergrund und Akzent anpassen oder PNG/JPEG importieren. Speichern übernimmt die Vorschau; Abbrechen stellt das bisherige Thema wieder her. Bilder bleiben lokal, werden begrenzt und im Hintergrund verarbeitet. Enthalten sind auch Profilversionsschutz und MCP-Zugriffskontrollen.

PNG / JPEG · bis 16 MiB und 16 Megapixel. Kopie in diesem Profil.
<!-- /custom-themes-3.4.0 -->

## Downloads & Wiederherstellung

### Downloads & Wiederherstellung

Ein Doppelklick auf einen Download öffnet Fortschritt, Geschwindigkeit und Restzeit. Pause und Stopp behalten gespeicherte Daten. Sofort starten gibt ausgewählten Dateien Vorrang. Fortsetzen erfordert Serverunterstützung und passende Dateidaten. Abgelaufene Links lassen sich im Kontextmenü aktualisieren. Fertige entfernen löscht nur Verlaufseinträge, nicht die gespeicherten Dateien. Aufgaben → Liste exportieren… speichert Ihre Downloads in einer Datei, als Sicherung oder für einen anderen PC. Aufgaben → Liste importieren… zeigt vor dem Hinzufügen, was neu ist; vorhandene Links werden übersprungen, neue Downloads bleiben pausiert. Ansicht → Schwebendes Ablagefeld zeigt ein kleines Feld, das über anderen Fenstern bleibt; legen Sie Links darauf ab, um sie herunterzuladen, auch wenn Download Engine im Infobereich ist.

## Offline-Websites

### Offline-Websites

Grabber sucht Dateilinks oder speichert eine Website offline. URL, Tiefe, Seiten- und Größenlimit wählen und Ergebnisse vor dem Hinzufügen prüfen. Gespeicherte index.html zum Browsen öffnen. Dynamische Funktionen, Anmeldung, Formulare und verschlüsselte Videos sind nicht unterstützt. Fehlende Ressourcen stehen im Bericht.

## Browserintegration

### Browserintegration

Unter Werkzeuge → Browsereinrichtung Chrome, Edge, Brave oder Firefox verbinden. Chrome, Edge und Brave benötigen die tatsächliche ID der installierten Erweiterung. In der Erweiterung Verbindung prüfen wählen. Browseroptionen enthält eine eigene Sprachauswahl. Die Videoschaltfläche öffnet die Qualitätsauswahl. Website-Anmeldung wird nur nach Ihrer Zustimmung für die ausgewählte Datei verwendet.

## Warteschlangen & Zeitpläne

### Warteschlangen & Zeitpläne

Unter Warteschlangen verwalten Dateien gruppieren und gleichzeitige Downloads einstellen. Neue Warteschlangen warten auf Start oder Zeitplan. Der Zeitplaner bietet Start, Pause und Wiederholungen. Die App muss geöffnet bleiben, auch im Infobereich. Automatisches Beenden, Herunterfahren, Energiesparen oder Ruhezustand muss gesondert aktiviert werden; der Countdown lässt sich abbrechen.

## Skripte

Editor und geladene Skripte haben jetzt eigene Registerkarten. Skripte lassen sich in der Liste suchen und erstellen; der vollständige Editor öffnet sich im Kopfbereich des Lua- oder PowerShell-Editors. Speichern, Ausführen/Stoppen und optionale Protokolle bleiben beim Code. Dateibefehle stehen im Aktionsmenü des Editors. Beim Ansichtswechsel bleiben ungespeicherter Code, Einstellungen und Cursorposition erhalten.


### Skripte

Unter Werkzeuge → Skripte Regeln nach Website oder Dateiendung erstellen. Unter Skripte Lua oder PowerShell wählen, speichern und ausführen. Auslöser: manuell, App-Start, Intervall, Download- oder Warteschlangenabschluss. Ein Skript überlappt seine eigenen Ausführungen nicht. Stopp beendet das Skript, macht aber bereits gesendete Downloadbefehle nicht rückgängig. Ausgabe steht im Ausführungsprotokoll.

## Lua-Skripte

### Lua-Skripte

Lua ist integriert. Neues Lua-Skript mit manuellem Auslöser erstellen, Beispiel speichern und ausführen. xd.queues() liefert Warteschlangen, xd.downloads() Downloads. Downloadabschluss liefert event.downloads, Warteschlangenabschluss event.queue. Befehle und Feldnamen nicht übersetzen. Vollständige API: docs/automation.md. Keine Geheimnisse protokollieren. PowerShell läuft mit Ihren Windows-Kontorechten.

Die Lua- und PowerShell-Editoren prüfen die Syntax nach einer kurzen Schreibpause automatisch, ohne das Skript zu speichern oder auszuführen. Neu sind Zeilennummern, Syntaxfarben und anklickbare Fehler. Ein Namensanfang oder Ctrl+Space öffnet Vorschläge: mit Pfeiltasten wählen, mit Tab, Enter oder Klick einfügen, mit Esc schließen. Ergänzungen lassen sich rückgängig machen; Laufzeitfehler stehen weiterhin im Protokoll.

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

Der Auslöser „Download fehlgeschlagen“ führt ein Skript aus, wenn ein Download nach den automatischen Wiederholungen von Download Engine weiter Handlungsbedarf hat; event.downloads enthält dann den error jedes Downloads. xd.retry(id) und xd.restart(id) versuchen es erneut (Skripte dürfen das 5-mal pro Download), xd.notify(title, text) zeigt eine Windows-Benachrichtigung, und xd.store.get und xd.store.set behalten Werte zwischen Ausführungen. xd.rename, xd.move und xd.remove ändern die Datei nur, wenn „Darf heruntergeladene Dateien ändern“ aktiviert ist. „Mit Beispiel ausführen…“ probiert ein Ereignisskript mit ausgewählten Downloads aus; Änderungen werden nur simuliert.

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

Um ein Skript in einem anderen Programm zu bearbeiten, verwenden Sie „Datei öffnen…“ und „In Datei speichern…“ in der Kopfzeile des Editors. „Datei öffnen…“ kopiert eine .lua- oder .ps1-Datei in den Editor und fragt vorher nach, wenn dabei ungespeicherter Code ersetzt würde; mit „Skript speichern“ behalten Sie sie. „In Datei speichern…“ schreibt den Code im Editor in eine Datei.

### Lua Core API 1

Erstelle oder öffne ein Skript unter Loaded scripts; wähle Lua und Manual only in Script settings. Full editor vergrößert den Editor. Save script speichert, Run script startet, Run logs zeigt die Ausgabe. Core API listet Funktionen für den aktuellen Auslöser; Ctrl+Space öffnet Vorschläge. API 1 bezeichnet den Vertrag, nicht die App-Version. Das Beispiel verarbeitet nur HTML im Speicher, ohne Netzwerk oder Downloads; Seitenabruf ist beim manuellen Skript false.

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


## Befehlszeile

### Befehlszeile

PowerShell im Anwendungsordner öffnen und folgende Befehle verwenden. Die App muss zum Annehmen von Steuerbefehlen laufen. JSON und Befehlsnamen bleiben Englisch. Pfade mit Leerzeichen in Anführungszeichen setzen.

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

## MCP-Integration

### MCP-Integration

Auf der MCP-Seite lokalen HTTP-Server starten und Clientkonfiguration kopieren. Mit Nur ansehen beginnen. Der KI-Client benötigt URL und privaten Authentifizierungsheader. Token nicht veröffentlichen. Server stoppen oder Token erneuern, um Zugriff zu widerrufen. Details: docs/mcp-ai-guide.md.

MCP öffnet sich jetzt als Popup über die Symbolleiste, das Menü Werkzeuge oder die Einstellungen. Die aktuelle Seite bleibt erhalten. Schließen mit × oder Escape lässt den Server weiterlaufen und behält ungespeicherte MCP-Änderungen bei. Zum Trennen der KI-Clients den Server stoppen. Beim Öffnen aus den Einstellungen kehrt das Schließen zum selben Entwurf zurück.

## Virenscans

Scanner: Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV.

Erfordert die englische Antivirusprogramm-Explorerintegration. Das Zeitlimit gilt für die Übergabe, nicht die Prüfung. Prüfungen in Antivirusprogramm abbrechen.

Der Standardinstallationsordner ist erforderlich.

Schutz und Quarantäne werden vom Antivirusprogramm gesteuert. Eine Anfrage bestätigt keine Bedrohungsfreiheit.

Einrichtung prüfen: Integration gefunden; die Scan-Verfügbarkeit wird bei der Anfrage geprüft.

### Virenscans

Öffnen Sie Werkzeuge → Virenscan. Einzeln wählt einen abgeschlossenen Download, Stapel mehrere Dateien. Einzelne Scans können abgebrochen werden. Während eines Scans oder der Malwarebytes / Kaspersky / Bitdefender-Prüfung sind die Registerkarten gesperrt.

Scanner in Einstellungen wählen, dann Werkzeuge → Virenscan → Stapel öffnen. Fertige Dateien auswählen, starten und Ergebnisse prüfen. Stapel stoppen verwirft noch nicht gesendete Anfragen. Malwarebytes / Kaspersky / Bitdefender wartet nach jeder Übergabe auf Ihre Prüfung. Übergebene Scans in Malwarebytes / Kaspersky / Bitdefender selbst stoppen. Eine gesendete Anfrage bedeutet nicht, dass die Datei sicher ist.

## Einstellungen & Protokolle

### Einstellungen & Protokolle

Einstellungen enthält Sprache, Design, Ordner, Geschwindigkeit, Datenkontingente, Proxy und Verbindungszahl. Protokolle lassen sich durchsuchen, filtern, kopieren und exportieren. URLs, Pfade, Skriptausgabe und technische Fehler behalten ihren ursprünglichen Text. Berichte vor dem Teilen prüfen.

Die installierte Version fragt bei jedem Start den aktuellen Windows-Downloadordner ab, auch nach einem Umzug auf ein anderes Laufwerk. Neue Downloads verwenden diesen Ort. Vorhandene Downloads, gespeicherter Fortschritt und selbst gewählte Ordner bleiben unverändert. Die portable Version bestimmt ihren Standardordner bei jedem Start relativ zur App. Ältere Profile behalten den Unterordnernamen IDM EX. Wenn der Ort nicht ermittelt werden kann, erscheint ein Fehler.

## Fehlerbehebung

### Fehlerbehebung

Bei Verbindungsproblemen laufende App, Erweiterungs-ID und Browserregistrierung prüfen. Nach Aktualisierung Erweiterung und Seite neu laden. Bei Downloadfehlern Speicherplatz, Ordnerrechte, Proxy und Linkgültigkeit prüfen. Wiederherstellungsdaten nicht vor einem Fortsetzungsversuch löschen. Bei fehlenden Schriftzeichen Windows-Sprachschriftarten ergänzen und App neu starten.
