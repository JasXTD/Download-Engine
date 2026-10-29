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
