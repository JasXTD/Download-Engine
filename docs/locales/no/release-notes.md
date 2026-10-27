## 3.1.0 — 2026-10-27

### Raskere videonedlasting og mindre belastning

Videoer laster ned opptil 4 segmenter parallelt og følger lavere globale eller nettstedsspesifikke tilkoblingsgrenser. Pause, nytt forsøk og omstart beholder alle verifiserte ferdige segmenter, også de som ble ferdige i annen rekkefølge; uferdige segmenter lastes ned på nytt. Statusvinduet viser aktive segmenter og tilkoblinger. Bufret skriving, sjeldnere fremdriftsoppdateringer og gjenbruk av databasetilkoblinger reduserer belastningen. Pause venter fortsatt til skrivingen er ferdig, og hver mottatt datablokk registreres varig i kvoten.

### Tydeligere feilstatus

Feilstatusen heter nå «Mislyktes»; detaljer og automatiske forsøk er uendret.
