## 3.1.1 — 2026-10-27

### Snabbare slutförande av stora filer

HTTP/HTTPS- och FTP-hämtningar med en enda del på samma enhet verifieras och sparas nu utan att en andra fullständig kopia skrivs. Hämtningar med flera delar behåller den tidigare asynkrona sammansättningen i rätt ordning. Sparande på en annan enhet använder fortfarande säker kopiering. Kontrollsummor, avbrytande, skydd för befintliga filnamn och markeringen för Internetursprung finns kvar. Avbrutet sparande återställs från den sparade delen eller sammansättningsfilen utan ny hämtning eller dubbletter.
