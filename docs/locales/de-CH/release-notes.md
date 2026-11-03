## 3.10.0 — 2026-11-03

### macOS und Linux

Native Pakete für Apple-Silicon- und Intel-Macs sowie ein Linux-x64-Archiv nutzen dieselbe Oberfläche und Download-Engine wie Windows. Enthalten sind FFmpeg mit Quellcode und Lizenzen, native Dateidialoge und die Zugangsdatenverwaltung des Betriebssystems. Mac-Pakete sind ad-hoc signiert und nicht von Apple notarisiert. Voraussetzungen und Testgrenzen stehen im Leitfaden.

[Guide / English](../../cross-platform.md)

## 3.9.0 — 2026-11-01

### Torrent-Ablauf

Magnet-Links aus dem Browser, abgelegte Torrent-Dateien und Vorschläge aus der Zwischenablage werden unterstützt. Seeding nach Abschluss und eingehende Verbindungen sind getrennte, standardmässig deaktivierte Optionen. Mehrere Dateien lassen sich prüfen sowie Ordner verschieben und umbenennen; Alle stoppen beendet auch die Freigabe.

[Guide / English](../../user-guide.md#download-torrents)

## 3.8.0 — 2026-11-01

### Torrents aus Windows öffnen

Öffnen Sie .torrent-Dateien aus dem Explorer und Magnet-Links aus dem Browser zur Prüfung. Die optionale Windows-Registrierung behält Standard-Apps bei; ausstehende Prüfungen bleiben nach einem Neustart erhalten. Der Torrent-Schutz wurde verstärkt: Magnete ohne zwischengespeicherte Metadaten benötigen einen Tracker, andernfalls verwenden Sie eine .torrent-Datei. Regressionstests prüfen den bestehenden Schutz vor Dateinamenskonflikten.

## 3.7.0 — 2026-11-01

### Torrent-Downloads

Torrent-Dateien und Magnet-Links mit Dateiauswahl, Pause, Fortsetzen und Wiederherstellung geprüfter Daten. Standardmässig ausgeschaltet; Aktivierung und Upload-Steuerung unter Einstellungen. Ein aktiver Proxy pausiert Torrents, nach Abschluss endet der Upload. Enthält die Korrekturen aus 3.6.1 und das Torrent-Hilfsprogramm.

### Release-Integration

Anleitung, README, Website und Installationsprüfungen wurden aktualisiert. Nach dem ersten hinzugefügten Torrent benötigt das Profil Version 3.7.0 oder neuer, damit ältere Versionen keine Torrent-Aufträge verändern. Das blosse Aktivieren der Funktion aktualisiert das Profil nicht.

## 3.6.1 — 2026-10-31

### Weniger Abhängigkeiten und Freigabe von Erweiterungen

Rusts integrierte Dateisperren ersetzen fs2; die alte winapi-Abhängigkeit entfällt. Sperr- und Warteverhalten bleiben erhalten. Erweiterungsupdates mit zusätzlichen Aufgaben, auch durch den Austausch einer Aufgaben-ID, benötigen jetzt selbst bei deaktivierter Erweiterung eine Freigabe.

## 3.6.0 — 2026-10-31

### Erweiterungen

Die neue Seite „Erweiterungen“ ersetzt „Skripte“ und bietet die Tabs Installiert, Skripte, Regeln und Ausführungsverlauf. Eine Erweiterung ist eine einzelne .de-extension.json-Datei, die Seitenleser, Downloadanbieter und Automatisierungsaufträge bündelt. Die Installation ist nur aus einer lokalen Datei möglich; Herausgeber werden nicht verifiziert. Eine Prüfung zeigt, was die Erweiterung kann. Sie startet deaktiviert, und ein Update muss vor neuem Zugriff um Zustimmung bitten. Ihr Webzugriff ist auf die angegebenen Hosts beschränkt. Nach der Installation der ersten Erweiterung können ältere App-Versionen dieses Profil nicht mehr beschreiben.

## 3.5.2 — 2026-10-30

### Beim Start nach Updates suchen

Standardmässig wird bei jedem Öffnen der App einmal geprüft. Bei einer neuen Version können Sie das Update herunterladen oder später fortfahren. Netzwerkfehler öffnen kein Fenster. Die Prüfung lässt sich in den Einstellungen ausschalten. Updates von der offiziellen GitHub-Releaseseite herunterladen und installieren.

## 3.5.1 — 2026-10-29

### Sicherheit für Lua-Anbieter

- Resolve/refresh und transform/finish laufen in Prozessen mit getrennten Berechtigungen. Verarbeitungsprozesse erhalten `xd.fetch` auch beim Laden des Quelltexts nicht. Nutzen Sie `xd.phase` zur phasenabhängigen Initialisierung; lokale Resolver-Variablen werden nicht übernommen.
- Gespeicherte Pläne enthalten nur Grösse, Hashes und Transformationskennung. Aufgelöste URLs werden bei Bedarf erneuert; der für einen Auftrag fixierte Quelltext wird durch DPAPI des aktuellen Windows-Benutzers geschützt. Die atomare Migration erhält Checkpoint-Bindungen und blockiert Schreibzugriffe älterer Versionen.
- `xd.key` und `scripts protect-key`, das Schlüssel aus umgeleitetem stdin liest, liefern undurchsichtige, an den Anbieter gebundene AES-Schlüssel-Handles. Geheime Schlüssel müssen nicht im Quelltext oder Export stehen. Alte Sicherungen und bereits im Bibliothekscode enthaltene Schlüssel werden nicht gelöscht; siehe `docs/lua-providers.md`.

### Verlauf leeren

Vor dem Leeren des Verlaufs erscheint eine Bestätigung mit Warteschlange und Anzahl abgeschlossener Downloads. Abbrechen oder Schliessen behält den Verlauf. Heruntergeladene Dateien bleiben erhalten.

## 3.5.0 — 2026-10-29

### Lua Provider API 1

Lua-Downloadanbieter lösen Quellen auf, verarbeiten Bytes und prüfen Hashes mit den üblichen Pause-/Fortsetzen-Funktionen. Jeder Download behält seine gespeicherte Skriptversion. Tests verwenden ausschliesslich lokal erzeugte verschlüsselte Dateien. Nach dem Speichern eines Anbieters können ältere App-Versionen dieses Profil nicht mehr beschreiben.

## 3.4.1 — 2026-10-29

Die Downloadliste vermeidet wiederholtes Kopieren und Suchen in grossen Warteschlangen. Kleine Fenster lassen mehr Platz für Dateinamen. Warteschlangen lassen sich neben ihrem Status starten oder pausieren; Starten übergeht den Zeitplan. Beim Überfahren des Status erscheinen Hinweise zu Wiederholungen und Fehlern. Übertragung, Prüfung und Fortsetzungsformat bleiben unverändert.

## 3.4.0 — 2026-10-29

### Themensammlung

Einstellungen → Darstellung → Eigene bietet Neon Courier, Reactor Night, Aurora und Moonveil. Hintergrund und Akzent anpassen oder PNG/JPEG importieren. Speichern übernimmt die Vorschau; Abbrechen stellt das bisherige Thema wieder her. Bilder bleiben lokal, werden begrenzt und im Hintergrund verarbeitet.

### Profil- und Steuerungsverbesserungen

Enthalten sind auch Profilversionsschutz und MCP-Zugriffskontrollen.
