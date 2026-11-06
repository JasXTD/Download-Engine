## 3.15.0 — 2026-11-06

### Nucleo dei provider Lua e sottotitoli ASS

HTTP in Lua supporta ora HEAD, PUT, PATCH, DELETE e OPTIONS, corpi binari con limiti di dimensione e sessioni fornite esplicitamente, vincolate a una sola origine. Le credenziali restano cifrate fuori dal resolver, non vengono salvate nei piani né inviate ai processori offline. I piani multimediali accettano segmenti di dimensione sconosciuta; lua_stream elabora blocchi limitati con punti di controllo verificati per pausa, ripresa, arresto e recupero.

Aggiunti l’importazione di credenziali binarie/RSA protette dal sistema operativo, byte casuali, SHA-1/SHA-256, HMAC, firma e decifratura RSA e decifratura di blocchi AES. I sottotitoli ASS mantengono stili e modifiche di formattazione interne in MKV, mentre MP4 li converte in sottotitoli testuali. Le nuove funzioni per credenziali e contenuti multimediali richiedono un’app che supporti la versione di scrittura del profilo 9. I formati dei punti di controllo esistenti restano compatibili.

Il laboratorio CENC offline per gli autori usa Lua per analizzare un sottoinsieme limitato della cifratura dei campioni AVC/AAC, simulare uno scambio di licenza firmato, trasferire le chiavi e unire le tracce decifrate ai sottotitoli ASS. Autenticazione dei siti, formati delle licenze e parser dei contenitori restano a carico del codice Lua. I test usano dati generati, senza servizi o account reali; non si tratta di un’implementazione CENC completa.

[English details](../../../CHANGELOG.md#3150--2026-11-06)

## 3.10.2 — 2026-11-04

### Rapporti di arresto anomalo locali

Guida e Registri consentono di esaminare, copiare ed esportare rapporti di Rust panic e chiusura anomala. Nessun invio automatico; password, URL di download, percorsi personali e dump di memoria sono esclusi.

[English details](../../../CHANGELOG.md#3102--2026-11-04)

## 3.10.1 — 2026-11-04

### Sicurezza e affidabilità

Migliorati i limiti delle risorse per la ricerca dei peer torrent, l’annullamento e il filtraggio degli indirizzi di rete. Aggiornata l’archiviazione delle credenziali su macOS e Linux preservando gli accessi salvati. Il salvataggio delle cartelle torrent non sovrascrive le destinazioni esistenti.

[English details](../../../CHANGELOG.md#3101--2026-11-04)

## 3.10.0 — 2026-11-03

### macOS e Linux

Pacchetti nativi per Mac Apple Silicon e Intel e un archivio Linux x64 con la stessa interfaccia e lo stesso motore di download di Windows. Sono inclusi FFmpeg, sorgenti e licenze, finestre di dialogo native e archiviazione delle credenziali tramite il sistema operativo. I pacchetti Mac hanno firma ad hoc, senza autenticazione notarile Apple. Requisiti e limiti dei test sono nella guida.

[Guide / English](../../cross-platform.md)

## 3.9.0 — 2026-11-01

### Gestione dei torrent

Supporto per link magnet dal browser, trascinamento dei file torrent e suggerimenti dagli appunti. La condivisione dopo il completamento e le connessioni in ingresso sono opzioni separate, disattivate per impostazione predefinita. Scansione di più file, spostamento e rinomina delle cartelle; Arresta tutto ferma anche la condivisione.

[Guide / English](../../user-guide.md#download-torrents)

## 3.8.0 — 2026-11-01

### Aprire torrent da Windows

Apri file .torrent da Esplora file e link magnet dal browser per verificarli. La registrazione facoltativa in Windows mantiene le app predefinite e le revisioni in attesa dopo il riavvio. La sicurezza dei torrent è stata rafforzata: i magnet senza metadati in cache richiedono un tracker, altrimenti usa un file .torrent. I test di regressione coprono la protezione esistente dai conflitti tra nomi di file.

## 3.7.0 — 2026-11-01

### Download torrent

File torrent e collegamenti magnet con selezione dei file, pausa, ripresa e recupero dei dati verificati. Disattivati per impostazione predefinita; attivazione e controllo degli invii nelle preferenze. Un proxy attivo sospende i torrent e gli invii terminano al completamento. Include le correzioni 3.6.1 e il programma ausiliario torrent.

### Integrazione della versione

Aggiornati la guida, il README, il sito e i controlli del programma di installazione. Dopo aver aggiunto il primo torrent, il profilo richiede la versione 3.7.0 o successiva per impedire alle versioni precedenti di modificare le attività torrent. La sola attivazione della funzione non aggiorna il profilo.

## 3.6.1 — 2026-10-31

### Riduzione delle dipendenze e approvazione delle estensioni

I blocchi dei file integrati in Rust sostituiscono fs2 ed eliminano la vecchia dipendenza winapi, mantenendo il comportamento di blocco e attesa. Gli aggiornamenti che aggiungono attività, anche sostituendone l'ID, richiedono ora l'approvazione anche quando l'estensione è disattivata.

## 3.6.0 — 2026-10-31

### Estensioni

La nuova pagina Estensioni sostituisce Script e offre le schede Installate, Script, Regole e Cronologia esecuzioni. Un'estensione è un unico file .de-extension.json che raggruppa lettori di siti, fornitori di download e attività di automazione. L'installazione è possibile solo da un file locale e i produttori non sono verificati. Una verifica mostra cosa può fare l'estensione, che parte disattivata, e un aggiornamento deve chiedere il permesso prima di ottenere nuovi accessi. L'accesso al web è limitato agli host dichiarati. Dopo l'installazione della prima estensione, le versioni precedenti non possono scrivere in questo profilo.

## 3.5.2 — 2026-10-30

### Controlla aggiornamenti all’avvio

Attivo per impostazione predefinita, controlla una volta a ogni apertura dell’app. Una nuova versione propone l’aggiornamento o il rinvio. Gli errori di rete non aprono finestre. Disattivabile nelle Preferenze. Scarica e installa gli aggiornamenti dalla pagina ufficiale delle versioni su GitHub.

## 3.5.1 — 2026-10-29

### Sicurezza dei provider Lua

- Resolve/refresh e transform/finish vengono eseguiti in processi con capacità separate. Il processore non riceve mai `xd.fetch`, neppure durante il caricamento del codice. Usare `xd.phase` per l’inizializzazione per fase; le variabili locali del resolver non vengono trasferite.
- I piani salvati conservano solo dimensione, hash e identificativo della trasformazione. Gli URL risolti vengono rinnovati quando necessario; il codice fissato per il processo di download è protetto da DPAPI per l’utente Windows corrente. La migrazione atomica mantiene i legami dei checkpoint e blocca le scritture delle versioni precedenti.
- `xd.key` e `scripts protect-key`, che legge le chiavi da stdin reindirizzato, forniscono handle di chiavi AES opachi e associati al provider. Non serve inserire chiavi segrete nel codice o nelle esportazioni. I vecchi backup e le chiavi già presenti negli script della libreria non vengono cancellati; vedere `docs/lua-providers.md`.

### Cancella cronologia

La cancellazione della cronologia richiede una conferma con la coda e il numero di download completati. Annullare o chiudere conserva la cronologia. I file scaricati rimangono nelle loro cartelle.

## 3.5.0 — 2026-10-29

### Lua Provider API 1

I fornitori Lua risolvono le sorgenti, elaborano i byte e verificano gli hash con i normali comandi di pausa e ripresa. Ogni download conserva la versione salvata dello script. I test usano solo file cifrati generati localmente. Dopo il salvataggio di un fornitore, le versioni precedenti non possono scrivere in questo profilo.

## 3.4.1 — 2026-10-29

L'elenco riduce copie e ricerche ripetute nelle code grandi. Le finestre piccole lasciano più spazio ai nomi dei file. Avvia o metti in pausa la coda accanto al suo stato; l'avvio ignora la pianificazione. Passa il puntatore sullo stato per leggere i dettagli di nuovi tentativi ed errori. Trasferimento, verifica e formato di ripresa restano invariati.

## 3.4.0 — 2026-10-29

### Raccolta temi

Preferenze → Aspetto → Personalizzato offre Neon Courier, Reactor Night, Aurora e Moonveil. Regola sfondo e accento o importa PNG/JPEG. Salva mantiene l’anteprima; Annulla ripristina il tema precedente. Le immagini restano locali, con limiti di dimensione ed elaborazione in background.

### Miglioramenti a profili e controlli

Inclusi anche protezione delle versioni del profilo e controlli di accesso MCP.
