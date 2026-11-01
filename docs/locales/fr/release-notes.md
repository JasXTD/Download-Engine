## 3.7.0 — 2026-11-01

### Téléchargements torrent

Fichiers torrent et liens magnet avec sélection des fichiers, pause, reprise et récupération des données vérifiées. Désactivés par défaut ; activation et contrôle des envois dans les préférences. Un proxy actif suspend les torrents et les envois cessent à la fin du téléchargement. Inclut les correctifs 3.6.1 et le programme auxiliaire torrent.

### Intégration de la version

Le guide, le README, le site et les vérifications du programme d’installation ont été mis à jour. Après l’ajout du premier torrent, le profil nécessite la version 3.7.0 ou ultérieure pour empêcher les anciennes versions de modifier les tâches torrent. Activer la fonction seule ne met pas le profil à niveau.

## 3.6.1 — 2026-10-31

### Réduction des dépendances et approbation des extensions

Les verrous de fichiers intégrés à Rust remplacent fs2 et suppriment l'ancienne dépendance winapi, sans changer le comportement des verrous et de l'attente. Les mises à jour ajoutant des tâches, y compris en remplaçant un identifiant de tâche, exigent désormais une approbation même si l'extension est désactivée.

## 3.6.0 — 2026-10-31

### Extensions

La nouvelle page Extensions remplace Scripts et propose les onglets Installées, Scripts, Règles et Historique d’exécution. Une extension est un seul fichier .de-extension.json qui regroupe des lecteurs de site, des fournisseurs de téléchargement et des tâches d’automatisation. L’installation se fait uniquement depuis un fichier local ; les éditeurs ne sont pas vérifiés. Une vérification indique ce que l’extension peut faire, elle démarre désactivée et une mise à jour doit demander l’autorisation avant tout nouvel accès. Son accès au web est limité aux hôtes déclarés. Après l’installation de la première extension, les anciennes versions ne peuvent plus écrire dans ce profil.

## 3.5.2 — 2026-10-30

### Rechercher les mises à jour au lancement

Activé par défaut, avec une vérification à chaque ouverture de l’application. Une nouvelle version propose une mise à jour ou un report. Les erreurs réseau n’ouvrent pas de fenêtre. Désactivation possible dans les Préférences. Téléchargez et installez les mises à jour depuis la page officielle des versions GitHub.

## 3.5.1 — 2026-10-29

### Sécurité des fournisseurs Lua

- Resolve/refresh et transform/finish s’exécutent dans des processus aux capacités séparées. Le processeur ne reçoit jamais `xd.fetch`, même au chargement du code. Utilisez `xd.phase` pour l’initialisation par phase ; les variables locales du résolveur ne sont pas transférées.
- Les plans enregistrés ne conservent que la taille, les empreintes et l’identifiant de transformation. Les URL résolues sont renouvelées au besoin ; le code figé pour une tâche est protégé par DPAPI pour l’utilisateur Windows actuel. La migration atomique préserve les liens des points de reprise et bloque les écritures des anciennes versions.
- `xd.key` et `scripts protect-key`, qui lit la clé depuis stdin redirigé, fournissent des handles de clés AES opaques liés au fournisseur. Les clés secrètes n’ont pas à figurer dans le code ou les exports. Les anciennes sauvegardes et les clés déjà écrites dans la bibliothèque ne sont pas effacées ; voir `docs/lua-providers.md`.

### Effacer l’historique

L’effacement de l’historique demande une confirmation indiquant la file et le nombre de téléchargements terminés. Annuler ou fermer conserve l’historique. Les fichiers téléchargés sont conservés.

## 3.5.0 — 2026-10-29

### Lua Provider API 1

Les fournisseurs Lua résolvent les sources, traitent les octets et vérifient les empreintes avec les commandes habituelles de pause et de reprise. Chaque téléchargement conserve sa version du script. Les tests utilisent uniquement des fichiers chiffrés générés localement. Après l’enregistrement d’un fournisseur, les anciennes versions ne peuvent plus écrire dans ce profil.

## 3.4.1 — 2026-10-29

La liste réduit les copies et recherches répétées dans les grandes files. Les petites fenêtres donnent plus de place aux noms de fichiers. Démarrez ou mettez en pause une file près de son état ; le démarrage ignore sa planification. Survolez un état pour lire les détails des nouvelles tentatives et erreurs. Le transfert, la vérification et le format de reprise restent inchangés.

## 3.4.0 — 2026-10-29

### Collection de thèmes

Préférences → Apparence → Personnalisé propose Neon Courier, Reactor Night, Aurora et Moonveil. Réglez le fond et l’accent ou importez un PNG/JPEG. Enregistrer conserve l’aperçu ; Annuler rétablit le thème précédent. Les images restent locales, avec des limites de taille et un traitement en arrière-plan.

### Améliorations des profils et des contrôles

Protection des versions du profil et contrôles d’accès MCP inclus.
