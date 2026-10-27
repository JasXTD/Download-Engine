## 3.1.0 — 2026-10-27

### Video più veloci e minore carico di trasferimento

I video scaricano fino a 4 segmenti in parallelo rispettando limiti globali o per sito inferiori. Pausa, nuovi tentativi e riavvio conservano tutti i segmenti completi verificati, anche terminati fuori ordine; quelli incompleti vengono riscaricati. La finestra di stato mostra segmenti e connessioni attivi. Scritture bufferizzate, aggiornamenti di avanzamento meno frequenti e riuso delle connessioni al database riducono il carico. La pausa attende ancora la fine delle scritture e ogni blocco ricevuto viene registrato in modo durevole nella quota.

### Stato di errore più chiaro

Lo stato di errore diventa “Non riusciti”; dettagli e tentativi automatici restano invariati.
