## 3.1.1 — 2026-10-27

### Grosse Dateien schneller fertigstellen

HTTP/HTTPS- und FTP-Downloads mit einem einzigen Teil auf demselben Laufwerk werden jetzt geprüft und gespeichert, ohne eine zweite vollständige Kopie zu schreiben. Mehrteilige Downloads behalten die bisherige asynchrone Zusammenfügung in der richtigen Reihenfolge bei. Beim Speichern auf einem anderen Laufwerk bleibt das sichere Kopieren erhalten. Prüfsummen, Abbruch, Schutz vorhandener Dateinamen und die Internet-Herkunftsmarkierung bleiben aktiv. Unterbrochenes Speichern wird aus dem gespeicherten Teil oder der Zusammenfügungsdatei ohne erneutes Herunterladen oder Duplikate wiederhergestellt.
