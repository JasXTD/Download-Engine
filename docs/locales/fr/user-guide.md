# Download Engine

## Premiers pas

Les préférences s’ouvrent dans une fenêtre organisée par catégories. Enregistrer applique les changements ; Annuler, × ou Échap abandonne les modifications non enregistrées. Le thème peut être prévisualisé ; Réinitialiser concerne uniquement la catégorie actuelle. Les éditeurs avancés enregistrent séparément et reviennent aux préférences une fois fermés.

### Premiers pas

Choisissez Ajouter une URL, collez un lien direct, vérifiez le nom, la file et le dossier puis démarrez. Ajouter un lot accepte un lien par ligne. « À partir d’un modèle » crée des liens numérotés comme img001…img100 à partir d’un lien contenant *. Changez la langue dans Paramètres → Apparence → Langue. Langue du système suit Windows ; les langues non prises en charge utilisent l’anglais.

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

## Scripts

### Scripts

Dans Outils → Scripts, créez des règles par site ou extension de fichier. Dans Scripts, choisissez Lua ou PowerShell, enregistrez et exécutez. Déclencheurs : manuel, démarrage, intervalle, fin de téléchargement ou de file. Un script ne chevauche pas ses propres exécutions. Arrêter termine le script sans annuler les commandes déjà envoyées. Consultez les journaux pour la sortie.

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
