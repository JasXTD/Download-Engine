## 3.1.1 — 2026-10-27

### Raskere fullføring av store filer

HTTP/HTTPS- og FTP-nedlastinger med én del på samme stasjon blir nå kontrollert og lagret uten å skrive en ekstra fullstendig kopi. Nedlastinger med flere deler beholder den tidligere asynkrone sammensettingen i riktig rekkefølge. Lagring på en annen stasjon bruker fortsatt sikker kopiering. Kontrollsummer, avbryting, beskyttelse av eksisterende filnavn og merking av Internett-opprinnelse beholdes. Avbrutt lagring gjenopprettes fra den lagrede delen eller sammensettingsfilen uten ny nedlasting eller duplikater.
