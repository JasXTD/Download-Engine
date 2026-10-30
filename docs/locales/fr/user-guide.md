# Download Engine

## Premiers pas

### Rechercher les mises à jour au lancement

Activé par défaut, avec une vérification à chaque ouverture de l’application. Une nouvelle version propose une mise à jour ou un report. Les erreurs réseau n’ouvrent pas de fenêtre. Désactivation possible dans les Préférences. Téléchargez et installez les mises à jour depuis la page officielle des versions GitHub.

Les préférences s’ouvrent dans une fenêtre organisée par catégories. Enregistrer applique les changements ; Annuler, × ou Échap abandonne les modifications non enregistrées. Le thème peut être prévisualisé ; Réinitialiser concerne uniquement la catégorie actuelle. Les éditeurs avancés enregistrent séparément et reviennent aux préférences une fois fermés.

### Premiers pas

Choisissez Ajouter une URL, collez un lien direct, vérifiez le nom, la file et le dossier puis démarrez. Ajouter un lot accepte un lien par ligne. « À partir d’un modèle » crée des liens numérotés comme img001…img100 à partir d’un lien contenant *. Changez la langue dans Paramètres → Apparence → Langue. Langue du système suit Windows ; les langues non prises en charge utilisent l’anglais.



<!-- custom-themes-3.4.0 -->
### Collection de thèmes

Préférences → Apparence → Personnalisé propose Neon Courier, Reactor Night, Aurora et Moonveil. Réglez le fond et l’accent ou importez un PNG/JPEG. Enregistrer conserve l’aperçu ; Annuler rétablit le thème précédent. Les images restent locales, avec des limites de taille et un traitement en arrière-plan. Protection des versions du profil et contrôles d’accès MCP inclus.

PNG / JPEG · jusqu’à 16 Mio et 16 mégapixels. Copie dans ce profil.
<!-- /custom-themes-3.4.0 -->

## Téléchargements et récupération

### Téléchargements et récupération

Double-cliquez sur un téléchargement pour voir progression, vitesse et temps restant. Pause et Arrêter conservent les données. Démarrer immédiatement donne la priorité aux fichiers choisis. La reprise dépend du serveur et de la concordance des données. Actualisez un lien expiré dans le menu contextuel. Effacer les terminés supprime seulement l’historique, pas les fichiers. Tâches → Exporter la liste… enregistre vos téléchargements dans un fichier, pour une sauvegarde ou un autre PC. Tâches → Importer une liste… montre les nouveautés avant de les ajouter ; les liens déjà présents sont ignorés et les nouveaux téléchargements restent en pause. Affichage → Zone de dépôt flottante affiche une petite zone qui reste au-dessus des autres fenêtres ; déposez-y des liens pour les télécharger, même quand Download Engine est dans la zone de notification.

## Sites hors ligne

### Sites hors ligne

Le Collecteur trouve des fichiers ou enregistre un site hors ligne. Définissez URL, profondeur, limites de pages et de taille, puis vérifiez les résultats avant ajout. Ouvrez index.html pour parcourir la copie. Fonctions dynamiques, connexion, formulaires et vidéos chiffrées non pris en charge. Le rapport indique les ressources manquantes.

## Intégration au navigateur

### Intégration au navigateur

Ouvrez Outils → Configuration du navigateur pour connecter Chrome, Edge, Brave ou Firefox. Chrome, Edge et Brave nécessitent l’ID réel de l’extension installée. Utilisez Vérifier la connexion dans l’extension. Options du navigateur possède sa propre langue. Le bouton vidéo ouvre le choix de qualité. La connexion d’un site sert uniquement au fichier approuvé.

## Files et planifications

### Files et planifications

Gérer les files permet de grouper les fichiers et limiter les téléchargements simultanés. Les nouvelles files attendent un démarrage ou un planning. Le planificateur gère début, pause et répétition. L’application doit rester ouverte, même dans la zone de notification. Fermeture, arrêt, mise en veille et veille prolongée automatiques exigent une activation séparée ; le compte à rebours peut être annulé.

## Extensions

### À quoi servent les extensions

Les extensions ajoutent la prise en charge de sites et l’automatisation en un seul paquet. Elles s’exécutent dans les mêmes processus Lua restreints que vos scripts, et chacune indique les sites auxquels elle peut accéder. Elles sont distinctes de l’extension de navigateur que vous chargez dans votre navigateur.

### Installer une extension

1. Ouvrez Extensions dans la barre d’outils supérieure. L’onglet Installées liste vos extensions.

2. Choisissez Installer depuis un fichier… et sélectionnez un fichier .de-extension.json.

3. Lisez la fenêtre de vérification. Elle indique l’éditeur, ce que l’extension peut faire et ses tâches. Rien ne s’exécute pour l’instant.

4. Cochez l’accès aux fichiers uniquement si vous voulez l’autoriser, puis choisissez Installer.

5. L’extension s’installe désactivée. Sélectionnez-la et cochez Activé le moment venu.

Les éditeurs ne sont pas vérifiés : installez uniquement des extensions provenant de personnes en qui vous avez confiance.

### Paramètres, code et mises à jour

Sélectionnez une extension pour voir ce qu’elle peut faire, ce qu’elle ne peut pas faire et quand ses tâches se sont exécutées pour la dernière fois. Paramètres contient ses options. Les valeurs protégées peuvent être remplacées ou effacées, mais ne peuvent plus être affichées. Code affiche ses fichiers en lecture seule. Copier dans mes scripts crée une copie modifiable sous forme de script personnel.

Pour mettre à jour, choisissez Installer depuis un fichier… et sélectionnez un fichier plus récent. La fenêtre de vérification indique ce qui a changé. Si la mise à jour demande un nouvel accès, vous devez l’approuver, et Pas maintenant laisse la version installée telle quelle. Les paramètres, clés et valeurs enregistrées sont conservés, et les téléchargements en cours gardent le code avec lequel ils ont démarré.

Supprimer efface les paramètres, clés et valeurs enregistrées de l’extension. L’historique d’exécution reste et les fichiers que vous avez téléchargés sont conservés.

### Script ou extension ?

Écrivez un script pour un besoin qui ne concerne que vous. Créez une extension pour ce que vous voulez partager ou maintenir à jour. Consultez docs/extensions.md pour le format du paquet.

## Scripts

L’éditeur et les scripts chargés disposent désormais d’onglets distincts. Recherchez ou créez un script dans la liste, puis ouvrez l’éditeur complet depuis l’en-tête Lua ou PowerShell. Enregistrement, exécution/arrêt et journaux facultatifs restent près du code. Les commandes de fichiers sont regroupées dans le menu d’actions de l’éditeur. Le changement de vue conserve le code non enregistré, les réglages et la position du curseur.


### Scripts

Dans Extensions → Règles, créez des règles par site ou extension de fichier. Dans Extensions → Scripts, choisissez Lua ou PowerShell, enregistrez et exécutez. Déclencheurs : manuel, démarrage, intervalle, fin de téléchargement ou de file. Un script ne chevauche pas ses propres exécutions. Arrêter termine le script sans annuler les commandes déjà envoyées. Consultez les journaux pour la sortie.

## Scripts Lua

### Scripts Lua

Lua est intégré. Créez un script Lua manuel, enregistrez cet exemple et exécutez-le. xd.queues() renvoie les files et xd.downloads() les téléchargements. La fin d’un téléchargement fournit event.downloads, celle d’une file event.queue. Ne traduisez pas les commandes et champs. API complète : docs/automation.md. Ne journalisez pas de secrets. PowerShell utilise les droits de votre compte Windows.

Les éditeurs Lua et PowerShell vérifient automatiquement la syntaxe après une courte pause de saisie, sans enregistrer ni exécuter le script. Ajout des numéros de ligne, de la coloration et des erreurs cliquables. Saisissez un début de nom ou Ctrl+Space pour les suggestions : flèches pour choisir, Tab, Enter ou clic pour insérer, Esc pour fermer. L’insertion peut être annulée ; les erreurs d’exécution restent dans les journaux.

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

Le déclencheur Échec du téléchargement exécute un script quand un téléchargement nécessite encore une intervention après les nouvelles tentatives automatiques de Download Engine ; event.downloads contient alors l’error de chaque téléchargement. xd.retry(id) et xd.restart(id) réessaient (un script peut le faire 5 fois par téléchargement), xd.notify(title, text) affiche une notification Windows, et xd.store.get et xd.store.set conservent des valeurs d’une exécution à l’autre. xd.rename, xd.move et xd.remove ne modifient le fichier que si Peut modifier les fichiers téléchargés est coché. Exécuter avec un exemple… essaie un script d’événement avec les téléchargements choisis ; les modifications sont seulement simulées.

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

Pour modifier un script dans un autre programme, utilisez Ouvrir un fichier… et Enregistrer dans un fichier… dans l’en-tête de l’éditeur. Ouvrir un fichier… copie un fichier .lua ou .ps1 dans l’éditeur et demande d’abord s’il remplacerait du code non enregistré ; choisissez Enregistrer le script pour le conserver. Enregistrer dans un fichier… écrit le code de l’éditeur dans un fichier.

### Lua Core API 1

Créez ou ouvrez un script dans Loaded scripts ; choisissez Lua et Manual only dans Script settings. Full editor agrandit l’éditeur. Utilisez Save script puis Run script et consultez Run logs. Core API liste les fonctions du déclencheur actuel ; Ctrl+Espace ouvre les suggestions. API 1 désigne le contrat, pas la version de l’application. Cet exemple analyse uniquement du HTML en mémoire, sans réseau ni téléchargement ; la lecture de pages vaut false pour un script manuel.

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


### Lua Provider API 1

Les fournisseurs Lua résolvent les sources, traitent les octets et vérifient les empreintes avec les commandes habituelles de pause et de reprise. Chaque téléchargement conserve sa version du script. Les tests utilisent uniquement des fichiers chiffrés générés localement. Après l’enregistrement d’un fournisseur, les anciennes versions ne peuvent plus écrire dans ce profil.

Extensions → Scripts → Script settings → Download provider → Save script → Add provider download.

API: `resolve(request)`, `refresh(request, previous)`, `transform(plan, bytes, offset)`, `finish(plan, result)`.

`docs/lua-providers.md` · `examples/download-provider.lua` · `examples/download-provider-custom-ctr.lua`

## Ligne de commande

### Ligne de commande

Ouvrez PowerShell dans le dossier de l’application et utilisez les commandes suivantes. L’application doit fonctionner pour recevoir les commandes de contrôle. JSON et noms de commandes restent anglais. Mettez les chemins contenant des espaces entre guillemets.

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

## Intégration MCP

### Intégration MCP

Sur la page MCP, démarrez le serveur HTTP local et copiez la configuration client. Commencez en lecture seule. Le client IA a besoin de l’URL et de l’en-tête privé d’authentification. Ne publiez pas le jeton. Arrêtez le serveur ou renouvelez le jeton pour révoquer l’accès. Détails : docs/mcp-ai-guide.md.

MCP s’ouvre désormais dans une fenêtre depuis la barre d’outils, le menu Outils ou les Préférences, sans changer de page. La fermeture avec × ou Escape laisse le serveur actif et conserve les modifications MCP non enregistrées. Arrêtez le serveur pour déconnecter les clients IA. Depuis les Préférences, la fermeture revient au même brouillon.

## Analyses antivirus

Analyseur: Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV.

Nécessite l’intégration Explorer de antivirus en anglais. Le délai limite le transfert, pas l’analyse. Annulez les analyses dans antivirus.

Le dossier d’installation par défaut est requis.

L’antivirus gère la protection et la quarantaine. Une demande ne confirme pas l’absence de menace.

Vérifier la configuration: Intégration trouvée ; la disponibilité de l’analyse est vérifiée à la demande.

### Analyses antivirus

Ouvrez Outils → Analyse antivirus. Individuel permet de choisir un téléchargement terminé, Lot plusieurs fichiers. Une analyse individuelle peut être annulée. Les onglets sont verrouillés pendant l’analyse ou la vérification dans Malwarebytes / Kaspersky / Bitdefender.

Choisissez l’antivirus dans les paramètres puis Outils → Analyse antivirus → Lot. Sélectionnez les fichiers terminés, démarrez et vérifiez les résultats. Arrêter le lot annule les demandes non envoyées. Malwarebytes / Kaspersky / Bitdefender attend votre vérification après chaque transfert. Arrêtez les analyses déjà transmises dans Malwarebytes / Kaspersky / Bitdefender. Une demande envoyée ne prouve pas qu’un fichier est sûr.

## Paramètres et journaux

### Paramètres et journaux

Les paramètres incluent langue, thème, dossiers, vitesse, quotas, proxy et connexions. Les journaux peuvent être recherchés, filtrés, copiés et exportés. URL, chemins, sorties de scripts et erreurs techniques gardent leur texte initial. Vérifiez les rapports avant de les partager.

La version installée demande à Windows son dossier Téléchargements actuel à chaque démarrage, même après un déplacement vers un autre disque. Les nouveaux téléchargements utilisent cet emplacement. Les téléchargements existants, leur progression et les dossiers choisis restent inchangés. La version portable détermine son dossier par rapport à l’application à chaque démarrage. Les anciens profils conservent le sous-dossier IDM EX. Une erreur apparaît si l’emplacement est introuvable.

## Dépannage

### Dépannage

En cas de connexion impossible, vérifiez l’application ouverte, l’ID d’extension et l’enregistrement du navigateur. Après mise à jour, rechargez extension et page. Pour les erreurs de téléchargement, vérifiez espace disque, droits du dossier, proxy et validité du lien. Ne supprimez pas les données de récupération avant d’essayer de reprendre. Si les caractères manquent, ajoutez les polices de langue Windows et redémarrez.
