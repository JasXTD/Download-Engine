## 3.1.0 — 2026-10-27

### Snabbare videohämtningar och mindre belastning

Videor hämtar upp till 4 segment parallellt och följer lägre globala eller webbplatsspecifika anslutningsgränser. Paus, nytt försök och omstart behåller alla verifierade färdiga segment, även de som slutförts i annan ordning; ofärdiga segment hämtas på nytt. Statusfönstret visar aktiva segment och anslutningar. Buffrade skrivningar, färre uppdateringar av förloppet och återanvända databasanslutningar minskar belastningen. Paus väntar fortfarande på att skrivningar slutförs och varje mottaget datablock bokförs beständigt mot kvoten.

### Tydligare felstatus

Felstatusen heter nu ”Misslyckades”; detaljer och automatiska försök är oförändrade.
