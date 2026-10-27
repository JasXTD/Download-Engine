## 3.1.1 — 2026-10-27

### Completamento più rapido dei file grandi

I download HTTP/HTTPS e FTP in una sola parte sulla stessa unità vengono verificati e salvati senza scrivere una seconda copia completa. I download in più parti mantengono il precedente assemblaggio asincrono nell’ordine corretto. Il salvataggio su un’altra unità mantiene la copia sicura. Restano attivi verifica del checksum, annullamento, protezione dei nomi esistenti e indicazione della provenienza da Internet. Un salvataggio interrotto viene recuperato dalla parte salvata o dal file di assemblaggio, senza scaricare di nuovo o creare duplicati.
