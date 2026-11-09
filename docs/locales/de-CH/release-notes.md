## 3.19.1 — 2026-11-10

### Dailymotion-Erkennung und Videovorschau

- Die Dailymotion-Erkennung und die Schaltfläche zum Herunterladen wurden korrigiert. Bei einem unbrauchbaren Link versucht die App eine alternative Wiedergabeliste desselben Videos. Verbesserte Diagnoseprotokolle enthalten keine vertraulichen Daten.

[English details](../../../CHANGELOG.md#3191--2026-11-10)

## 3.19.0 — 2026-11-08

### BitTorrent v2 und hybride Torrents

- Unterstützung für v2- und Hybrid-Torrents mit SHA-256-Prüfung, Dateiauswahl, Fortsetzen und Wiederherstellung. V1 und die bisherigen Freigabeeinstellungen bleiben erhalten.

[English details](../../../CHANGELOG.md#3190--2026-11-08)

## 3.18.1 — 2026-11-08

- Uploads sind standardmässig ausgeschaltet. Sie können sie unter Einstellungen → Torrents aktivieren.
- **Eingehende Verbindungen erlauben?** Andere Peers können sich mit dieser App verbinden. Dadurch ist sie stärker dem Netzwerk ausgesetzt. Ihre Firewall- und Routereinstellungen bleiben unverändert.
- **UPnP aktivieren?** Während Torrents laufen, wird Ihr Router um eine Weiterleitung des Torrent-TCP-Ports gebeten. Andere Peers können diese App dann möglicherweise aus dem Internet erreichen. Speichern Sie die Einstellungen, um diese Änderung anzuwenden.

[English details](../../../CHANGELOG.md#3181--2026-11-08)

## 3.18.0 — 2026-11-08

### VPN / Netzwerkschnittstelle

- Torrents an eine Schnittstellenadresse binden. Fällt sie weg, stoppen die Übertragungen; andere Downloads behalten ihre Einstellungen.
- DNS nutzt ebenfalls diese Schnittstelle. Verwenden Sie dieselbe IP-Version wie die gewählte Adresse. Router-Portfreigaben sind während der Bindung deaktiviert.
- TCP / uTP · Verschlüsselt / Klartext

[English details](../../../CHANGELOG.md#3180--2026-11-08)

## 3.17.0 — 2026-11-08

### Peer-Verschlüsselung, uTP und Webseeds

Optionale MSE/PE-Peer-Verschlüsselung, uTP-Verbindungen und HTTP/HTTPS-Webseeds hinzugefügt. Alle drei sind standardmässig aus und können in den Torrent-Einstellungen aktiviert werden. Teile werden geprüft und Quellen für die Wiederaufnahme nach einem Neustart gespeichert. Pause, Fortsetzen und Alle stoppen wurden verbessert. Die Verschlüsselung verbirgt die IP-Adresse nicht.

[English details](../../../CHANGELOG.md#3170--2026-11-08)

## 3.16.1 — 2026-11-08

### Korrekturen an Einstellungen und Steuerung

Erweiterungseinstellungen speichern leeren Text und das gesamte Formular atomar. Ungespeicherte Änderungen bleiben beim Wechsel erhalten; die Schaltfläche zum Speichern bleibt in kleinen Fenstern sichtbar. Updates laden Standardwerte neu. Stoppmenüs für geteilte Torrents und die Anzeige von Geschwindigkeitslimits wurden korrigiert, wiederholte Fortschrittsschreibvorgänge reduziert. Die Beschriftung zum Berichtexport wurde vereinfacht; Hinweise zum Systemschutz und zu Update-Einstellungen wurden berichtigt.

[English details](../../../CHANGELOG.md#3161--2026-11-08)

## 3.16.0 — 2026-11-07

### Lua-Beispielerweiterungen und Berechtigungen

Fünf Lua-Erweiterungen sind enthalten: öffentliche Internet-Archive-Dateien, Wikimedia-Commons-Originale, GitHub-Release-Dateien, Sortierung fertiger Dateien nach Typ und Benachrichtigungen zum Warteschlangenende mit optionalem Start der nächsten Warteschlange. Wählen Sie sie unter Extensions → Examples und prüfen Sie die Berechtigungen vor der Installation. Mitgelieferter Code läuft nicht automatisch. Die leere Erweiterungsseite, Installationsprüfung und Anzeige der Auftragsrechte wurden verbessert. Dateizugriff lässt sich nachträglich gewähren oder entziehen; eine Anleitung erklärt die erste eigene Erweiterung.

Geänderte Pakete erfordern eine erneute Prüfung. Reservierte Beispiel-IDs setzen exakt die mitgelieferten Dateien voraus; irreführende Steuerzeichen in Metadaten werden abgewiesen. Erweiterungen werden weiterhin im plugins-Ordner der App gefunden. Fehlende Absturzbericht- und UI-Übersetzungen wurden ergänzt, die Erkennung übersetzbarer Texte verbessert und die rätoromanische Sprache vereinheitlicht. Fehlende ältere Versionshinweise wurden wiederhergestellt. macOS/Linux-Pakete enthalten nun die verlinkten Anleitungen und Bilder für die Offline-Dokumentation.

[English details](../../../CHANGELOG.md#3160--2026-11-07)

## 3.15.0 — 2026-11-06

### Lua-Provider und ASS-Untertitel

Lua-HTTP unterstützt nun HEAD, PUT, PATCH, DELETE und OPTIONS, begrenzte Binärdaten sowie ausdrücklich übergebene Sitzungen für genau einen Ursprung. Zugangsdaten bleiben ausserhalb des Resolvers verschlüsselt und gelangen weder in Pläne noch an Offline-Prozessoren. Medienpläne erlauben unbekannte Segmentgrössen; lua_stream verarbeitet begrenzte Datenblöcke mit geprüften Kontrollpunkten für Pause, Fortsetzen, Stopp und Wiederherstellung.

Hinzu kommen betriebssystemgeschützte Binär-/RSA-Zugangsdatenimporte, Zufallsbytes, SHA-1/SHA-256, HMAC, RSA-Signaturen und -Entschlüsselung sowie AES-Blockentschlüsselung. ASS-Untertitel behalten Stile und Inline-Formatierungen in MKV; MP4 wandelt sie in Textuntertitel um. Neue Zugangsdaten- und Medienfunktionen benötigen eine App mit Unterstützung für die Profil-Schreibversion 9. Bestehende Kontrollpunktformate bleiben kompatibel.

Das Offline-CENC-Labor für Autoren zeigt mit Lua einen begrenzten Teil der AVC/AAC-Sample-Verschlüsselung, einen simulierten signierten Lizenzaustausch, die Schlüsselübergabe und das Zusammenführen entschlüsselter Spuren mit ASS-Untertiteln. Website-Anmeldung, Lizenzformate und Container-Parser bleiben Aufgabe des Lua-Codes. Die Tests nutzen erzeugte Daten ohne echte Dienste oder Konten; dies ist keine vollständige CENC-Implementierung.

### Torrent-Freigabe und -Suche

Die Freigabe eines fertigen Torrents endet jetzt von selbst bei einem Verhältnis von 2.0 oder nach einer Woche, je nachdem, was zuerst eintritt. Das gilt standardmässig, auch für bestehende Profile; beide Grenzen lassen sich unter Einstellungen → Torrents ändern oder abschalten. Uploads während des Downloads zählen zum Verhältnis. Das Statusfenster zeigt das Verhältnis; ein durch eine Grenze gestoppter Torrent behält seine Dateien und bietet im Menü Freigabe erneut starten an.

Magnet-Links ohne Tracker werden nicht mehr einfach abgelehnt. Das Fenster Download hinzufügen erklärt, dass bei einer DHT-Suche andere Computer sehen, welchen Torrent Sie suchen, und Ihre IP-Adresse; gesucht wird erst nach einem Klick auf Im DHT suchen. Ihr Computer wird erst als Peer bekannt gegeben, wenn die geprüfte Dateiliste einen öffentlichen Torrent zeigt. Ein privater Torrent beendet die Suche, und eine Suche endet nach 10 Minuten. Links aus Browsern, der Zwischenablage oder dem Explorer starten nie von selbst eine Suche.

Neu: Router automatisch einrichten (UPnP), standardmässig ausgeschaltet wie die eingehenden Verbindungen, die es voraussetzt. Es spricht nur mit dem Standard-Gateway im privaten Netz, fordert eine Weiterleitungsregel für jeweils eine Stunde an und entfernt nur die eigene Regel; unter macOS ist es nicht verfügbar. Gefundene Peers und DHT-Knoten auf Ports anderer Dienste (unter 1024 sowie etwa SIP 5060) werden nie kontaktiert. Downloads, die aus einem DHT-Grund stoppen, nennen den Grund jetzt in der gewählten Sprache.

[English details](../../../CHANGELOG.md#3150--2026-11-06)

## 3.10.2 — 2026-11-04

### Lokale Absturzberichte

Hilfe und Protokolle bieten Berichte zu Rust-Panics und unerwartetem Beenden zum Prüfen, Kopieren und Exportieren. Kein automatischer Upload; Passwörter, Download-URLs, persönliche Pfade und Speicherabbilder sind ausgeschlossen.

[English details](../../../CHANGELOG.md#3102--2026-11-04)

## 3.10.1 — 2026-11-04

### Sicherheit und Zuverlässigkeit

Ressourcenlimits bei der Torrent-Peer-Suche, Abbruch und Netzwerkadressfilter wurden verbessert. Die Zugangsdaten-Speicherung unter macOS und Linux wurde unter Beibehaltung gespeicherter Anmeldungen aktualisiert. Vorhandene Ziele werden beim Speichern von Torrent-Ordnern nicht überschrieben.

[English details](../../../CHANGELOG.md#3101--2026-11-04)

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

## 3.3.3 — 2026-10-28

### Verbesserungen der Oberfläche

Editor und geladene Skripte haben jetzt eigene Registerkarten. Skripte lassen sich in der Liste suchen und erstellen; der vollständige Editor öffnet sich im Kopfbereich des Lua- oder PowerShell-Editors. Speichern, Ausführen/Stoppen und optionale Protokolle bleiben beim Code. Dateibefehle stehen im Aktionsmenü des Editors. Beim Ansichtswechsel bleiben ungespeicherter Code, Einstellungen und Cursorposition erhalten.

### Reaktionsschnellere Download-Aktionen

Warteschlangen- und Statusaktualisierungen, Videoanfragen des Browsers und das Hinzufügen von Downloads laufen im Hintergrund, damit langsame Datenträger keine Klicks blockieren. Dialoge zeigen den Fortschritt, verhindern doppelte Übermittlungen und behalten Eingaben bei Speicherfehlern. Auch Ordner öffnen sich im Hintergrund; ältere Ergebnisse überschreiben keine neu gewählte Warteschlange.
