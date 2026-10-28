# Download Engine

## Primi passi

Le preferenze si aprono in una finestra con categorie. Salva applica le modifiche; Annulla, × o Escape elimina quelle non salvate. Il tema può essere visualizzato in anteprima; Ripristina riguarda solo la categoria attuale. Gli editor avanzati salvano separatamente e tornano alle preferenze quando vengono chiusi.

### Primi passi

Scegli Aggiungi URL, incolla un link diretto e verifica nome file, coda e cartella, poi avvia. Aggiungi gruppo accetta un link per riga. Da uno schema genera link numerati come img001…img100 da un link con *. In Impostazioni → Aspetto → Lingua scegli Italiano oppure Lingua di sistema. La selezione automatica riconosce anche it-CH; le lingue non supportate usano l’inglese. La scelta si applica subito e viene conservata.



<!-- custom-themes-3.4.0 -->
### Raccolta temi

Preferenze → Aspetto → Personalizzato offre Neon Courier, Reactor Night, Aurora e Moonveil. Regola sfondo e accento o importa PNG/JPEG. Salva mantiene l’anteprima; Annulla ripristina il tema precedente. Le immagini restano locali, con limiti di dimensione ed elaborazione in background. Inclusi anche protezione delle versioni del profilo e controlli di accesso MCP.

PNG / JPEG · fino a 16 MiB e 16 megapixel. Copia in questo profilo.
<!-- /custom-themes-3.4.0 -->

## Download e recupero

### Download e recupero

Fai doppio clic su un download per vedere avanzamento, velocità e tempo rimasto. Pausa e Arresta conservano i dati parziali. Avvia subito dà priorità ai file selezionati; valgono comunque i limiti dell’app e le quote dati. Per un link scaduto usa Aggiorna link di download nel menu contestuale. Il recupero controlla i dati salvati prima di riprendere. Rimuovi completati cancella le voci della cronologia e conserva i file. Attività → Esporta elenco… salva i download in un file, come backup o per un altro PC. Attività → Importa elenco… mostra le novità prima di aggiungerle; i link già presenti vengono saltati e i nuovi download restano in pausa. Visualizza → Area di rilascio fluttuante mostra una piccola area che resta sopra le altre finestre; rilascia qui i link per scaricarli, anche quando Download Engine è nell’area di notifica.

## Siti offline

### Siti offline

Il Grabber trova file scaricabili oppure salva un sito offline. Imposta pagina iniziale, profondità, limite pagine e limite dimensione. Verifica i file trovati prima di aggiungerli alla coda. Per il sito offline apri index.html nella copia salvata. Script, moduli, accesso e lettori incorporati sono disattivati nella copia statica. Il rapporto di acquisizione indica le risorse mancanti.

## Integrazione browser

### Integrazione browser

Apri Strumenti → Configurazione browser per collegare Chrome, Edge, Brave o Firefox. Chrome, Edge e Brave richiedono l’ID reale dell’estensione installata. Nel popup dell’estensione usa Verifica connessione. Opzioni browser → Lingua dell’estensione offre una scelta indipendente dalla lingua dell’app. Il pulsante video apre la selezione di qualità nell’app. L’accesso del sito viene usato solo per il download approvato.

## Code e pianificazioni

### Code e pianificazioni

Gestisci code raggruppa i file e limita quanti download possono funzionare insieme. Le nuove code attendono l’avvio manuale o una pianificazione. Imposta orari di avvio e pausa e giorni di ripetizione nella Pianificazione. L’app deve restare aperta, anche nell’area di notifica. Uscita automatica, spegnimento, sospensione e ibernazione richiedono una scelta esplicita; puoi annullare il conto alla rovescia.

## Script

L’editor e gli script caricati hanno ora schede separate. Cerca o crea gli script nell’elenco e apri l’editor completo dall’intestazione Lua o PowerShell. Salvataggio, esecuzione/arresto e registri facoltativi restano accanto al codice. I comandi dei file sono nel menu delle azioni dell’editor. Il cambio di vista conserva codice non salvato, impostazioni e posizione del cursore.


### Script

In Strumenti → Script crea regole per sito o estensione del file. La prima regola di instradamento corrispondente viene applicata. Nella sezione Script puoi usare Lua o PowerShell. Salva le modifiche prima di eseguire. Gli eventi comprendono esecuzione manuale, avvio dell’app, intervallo, completamento di download o coda. Attiva e salva l’evento automatico per usarlo. Arresta script termina l’esecuzione senza annullare le azioni già inviate all’app. Consulta i log esecuzioni per risultati ed errori.

## Script Lua

### Script Lua

Lua 5.4 è integrato: non devi installarlo separatamente. Scegli Nuovo script, lingua Lua ed evento Solo manuale. Incolla l’esempio qui sotto, salva ed esegui. Nei log dovresti vedere Hello from Lua!, l’evento e i nomi delle code. xd.queues() restituisce le code; xd.downloads() restituisce i download. event.downloads contiene i file completati per l’evento di completamento; event.queue descrive la coda per il relativo evento. I nomi delle API e dei campi restano inglesi. L’API completa è in docs/automation.md. Non inserire segreti nei log. Lua usa un ambiente limitato; PowerShell ha i permessi del tuo account Windows.

Gli editor Lua e PowerShell controllano automaticamente la sintassi dopo una breve pausa nella digitazione, senza salvare né eseguire lo script. Aggiunti numeri di riga, colori della sintassi ed errori cliccabili. Digita l’inizio di un nome o premi Ctrl+Space per i suggerimenti: scegli con le frecce, inserisci con Tab, Enter o un clic e chiudi con Esc. Gli inserimenti si possono annullare; gli errori di esecuzione restano nei registri.

```lua
print("Hello from Lua!")
print("Trigger:", event.trigger)
for _, queue in ipairs(xd.queues()) do
    print(queue.name, "running:", queue.running)
end
for _, file in ipairs(event.downloads) do
    print("Finished:", file.name or file.id)
end
```

L'evento di attivazione Download non riuscito esegue uno script quando un download richiede ancora attenzione dopo i tentativi automatici di Download Engine; event.downloads contiene allora l'error di ogni download. xd.retry(id) e xd.restart(id) riprovano (uno script può farlo 5 volte per download), xd.notify(title, text) mostra una notifica di Windows e xd.store.get e xd.store.set conservano valori tra un'esecuzione e l'altra. xd.rename, xd.move e xd.remove modificano il file solo se Può modificare i file scaricati è selezionato. Esegui con un esempio… prova uno script di evento con i download che scegli; le modifiche sono solo simulate.

```lua
for _, file in ipairs(event.downloads) do
    if pcall(xd.retry, file.id) then
        print("Retrying:", file.name or file.id)
    else
        xd.notify("Download still failing", (file.name or file.id) .. ": " .. (file.error or "no details"))
    end
end
local handled = (xd.store.get("handled") or 0) + #event.downloads
xd.store.set("handled", handled)
print("Failed downloads handled:", handled)
```

Per modificare uno script in un altro programma, usa Apri file… e Salva su file… nell'intestazione dell'editor. Apri file… copia un file .lua o .ps1 nell'editor e chiede prima se sostituirebbe codice non salvato; scegli Salva script per mantenerlo. Salva su file… scrive il codice dell'editor in un file.

### Lua Core API 1

Crea o apri uno script in Loaded scripts; scegli Lua e Manual only in Script settings. Full editor amplia lo spazio. Scegli Save script, poi Run script e leggi Run logs. Core API elenca le funzioni del trigger corrente; Ctrl+Spazio apre i suggerimenti. API 1 indica il contratto, non la versione dell’app. L’esempio analizza solo HTML in memoria, senza rete né download; il recupero di pagine è false per uno script manuale.

```lua
-- Example: core-api
assert(xd.api_version == 1, "This example needs Lua API 1")
print("App:", xd.app_version, "Role:", xd.script_kind)
local html = '<a class="download" href="/files/demo.zip">Demo</a>'
local links = xd.html_select(html, "a.download[href]", "href")
local url = xd.url_join("https://example.com/catalog/", links[1])
print("Preview:", xd.to_json({ kind = "file", url = url }))
print("Can fetch a page:", xd.supports("xd.fetch"))
```

Open examples/core-api-automation.lua or examples/custom-site-reader.lua with Editor actions → Open file…. Site reader uses public pages, Pages patterns and Direct connection; test with Test on URL before enabling its trigger. Ordinary automation cannot call xd.fetch. Import script… accepts .de-script.json as a new draft with automatic triggers and file changes off; review its code and reselect local queues. Stop script does not undo submitted downloads. Full English reference: docs/automation.md.


## Riga di comando

### Riga di comando

Apri PowerShell nella cartella dell’applicazione e prova questi comandi di consultazione. L’app deve essere in esecuzione per ricevere i comandi di controllo. Nomi dei comandi, identificatori e JSON restano in inglese. Racchiudi tra virgolette i percorsi con spazi. Vedi docs/automation.md per aggiunte in gruppo e controllo delle code.

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

## Integrazione MCP

### Integrazione MCP

Nella pagina MCP avvia il server HTTP locale e copia la configurazione client nel tuo client AI. Inizia con Sola lettura. Il client richiede l’URL e l’intestazione privata Authorization. Non condividere il token. Consenti modifiche abilita le azioni sull’app, compresa l’esecuzione di script salvati. Arresta server scollega i client; i download già avviati continuano. Consulta docs/mcp-ai-guide.md per l’uso degli strumenti.

MCP si apre ora in una finestra dalla barra degli strumenti, dal menu Strumenti o dalle Preferenze, mantenendo la pagina corrente. Chiudere con × o Escape lascia attivo il server e conserva le modifiche MCP non salvate. Arrestare il server per disconnettere i client IA. Se aperta dalle Preferenze, la chiusura torna alla stessa bozza.

## Scansione antivirus

Scanner: Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV.

Richiede l'integrazione Explorer di antivirus in inglese. Il timeout limita l'invio della richiesta, non la scansione. Annulla le scansioni in antivirus.

È richiesta la cartella di installazione predefinita.

L’antivirus controlla protezione e quarantena. Una richiesta non conferma l’assenza di minacce.

Verifica configurazione: Integrazione trovata; la disponibilità della scansione viene verificata alla richiesta.

### Scansione antivirus

Apri Strumenti → Scansione antivirus. Singolo seleziona un download completato, Multiplo più file. Puoi annullare una scansione singola. Le schede sono bloccate durante la scansione o la verifica in Malwarebytes / Kaspersky / Bitdefender.

Scegli lo scanner nelle Impostazioni, poi apri Strumenti → Scansione antivirus → Multiplo. Seleziona i file completati, avvia e verifica i risultati. I file vengono analizzati uno alla volta. Interrompi gruppo annulla le richieste restanti. Con Malwarebytes / Kaspersky / Bitdefender attendi la fine della scansione e verifica il risultato nella sua app prima di passare al file successivo. Interrompi in Malwarebytes / Kaspersky / Bitdefender una scansione già consegnata. Una richiesta inviata non dimostra che il file sia privo di minacce.

## Impostazioni e log

### Impostazioni e log

Le Impostazioni comprendono lingua, tema, cartelle, velocità, quote, connessioni per file e proxy. Log permette di cercare, filtrare, copiare ed esportare gli eventi. URL, percorsi, output degli script ed errori tecnici conservano il testo originale. Rivedi il contenuto dei rapporti prima di condividerli. Le guide rapide sono disponibili offline; i riferimenti tecnici estesi e la cronologia precedente restano in inglese.

La versione installata chiede a Windows la cartella Download attuale a ogni avvio, anche dopo lo spostamento su un’altra unità. I nuovi download usano tale posizione. Download esistenti, avanzamento salvato e cartelle personalizzate restano invariati. La versione portatile determina la cartella in base alla posizione dell’app a ogni avvio. I vecchi profili mantengono la sottocartella IDM EX. Se la posizione non è disponibile, viene mostrato un errore.

## Risoluzione dei problemi

### Risoluzione dei problemi

Se la connessione al browser non funziona, verifica che l’app sia aperta, che l’ID dell’estensione sia corretto e che il browser sia registrato. Dopo l’aggiornamento ricarica estensione e pagina. Per errori di download controlla spazio libero, permessi della cartella, proxy e validità del link. Prova la ripresa prima di eliminare dati parziali. Se mancano caratteri, installa il supporto per i caratteri della lingua in Windows e riavvia l’app.
