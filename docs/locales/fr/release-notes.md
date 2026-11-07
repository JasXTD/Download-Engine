## 3.15.0 — 2026-11-06

### Noyau des fournisseurs Lua et sous-titres ASS

HTTP dans Lua prend désormais en charge HEAD, PUT, PATCH, DELETE et OPTIONS, des corps binaires limités et des sessions fournies explicitement, liées à une seule origine. Les identifiants restent chiffrés hors du résolveur et ne sont ni enregistrés dans les plans ni transmis aux processeurs hors ligne. Les plans multimédias acceptent des segments de taille inconnue ; lua_stream traite des blocs limités avec des points de contrôle vérifiés pour la pause, la reprise, l’arrêt et la récupération.

Ajout de l’import d’identifiants binaires/RSA protégés par le système, d’octets aléatoires, de SHA-1/SHA-256, de HMAC, de la signature et du déchiffrement RSA ainsi que du déchiffrement de blocs AES. Les sous-titres ASS conservent leurs styles et leurs modifications de formatage intégrées dans MKV ; MP4 les convertit en sous-titres texte. Les nouvelles fonctions d’identifiants et de médias nécessitent une application prenant en charge la version d’écriture des profils 9. Les anciens formats de points de contrôle restent compatibles.

Le laboratoire CENC hors ligne pour les auteurs utilise Lua pour analyser un sous-ensemble limité du chiffrement d’échantillons AVC/AAC, simuler un échange de licence signé, transmettre les clés et assembler les pistes déchiffrées avec des sous-titres ASS. L’authentification des sites, les formats de licence et les analyseurs de conteneurs restent à implémenter en Lua. Les tests utilisent des données générées, sans service ni compte réel ; il ne s’agit pas d’une implémentation complète de CENC.

### Partage et recherche de torrents

Le partage d’un torrent terminé s’arrête désormais de lui-même au ratio 2.0 ou après une semaine, selon ce qui arrive en premier. C’est activé par défaut, y compris pour les profils existants ; les deux limites se modifient ou se désactivent dans Préférences → Torrents. Les envois pendant le téléchargement comptent dans le ratio. La fenêtre d’état affiche le ratio, et un torrent arrêté par une limite garde ses fichiers et propose Partager à nouveau dans son menu.

Les liens magnet sans tracker ne sont plus simplement refusés. La fenêtre Ajouter un téléchargement explique qu’avec une recherche DHT, les autres ordinateurs voient quel torrent vous voulez et votre adresse IP, et ne lance la recherche qu’après un clic sur Chercher dans le DHT. Votre ordinateur n’est annoncé comme pair qu’une fois que la liste de fichiers vérifiée indique un torrent public. Un torrent privé arrête la recherche, et une recherche s’arrête après 10 minutes. Les liens venant des navigateurs, du presse-papiers ou de l’Explorateur ne lancent jamais de recherche d’eux-mêmes.

Nouveau : Configurer mon routeur (UPnP), désactivé par défaut comme les connexions entrantes dont il a besoin. Il ne communique qu’avec la passerelle par défaut du réseau privé, demande une seule règle de redirection valable une heure à la fois et ne supprime que sa propre règle ; macOS l’indique comme indisponible. Les pairs et nœuds DHT découverts sur des ports d’autres services (moins de 1024, ainsi que des ports comme SIP 5060) ne sont jamais contactés. Les téléchargements arrêtés pour une raison liée au DHT affichent désormais cette raison dans la langue choisie.

[English details](../../../CHANGELOG.md#3150--2026-11-06)

## 3.10.2 — 2026-11-04

### Rapports de plantage locaux

L’aide et les journaux permettent de consulter, copier et exporter les rapports de panique Rust et de fermeture anormale. Aucun envoi automatique ; les mots de passe, URL de téléchargement, chemins personnels et images mémoire sont exclus.

[English details](../../../CHANGELOG.md#3102--2026-11-04)

## 3.10.1 — 2026-11-04

### Sécurité et fiabilité

Amélioration des limites de ressources pour la recherche de pairs torrent, de l’annulation et du filtrage des adresses réseau. Mise à jour du stockage des identifiants sur macOS et Linux en préservant les connexions enregistrées. L’enregistrement des dossiers torrent ne remplace plus une destination existante.

[English details](../../../CHANGELOG.md#3101--2026-11-04)

## 3.10.0 — 2026-11-03

### macOS et Linux

Des paquets natifs pour Mac Apple Silicon et Intel et une archive Linux x64 utilisent la même interface et le même moteur de téléchargement que Windows. FFmpeg, ses sources et licences sont inclus, avec les boîtes de dialogue natives et le stockage sécurisé du système. Les paquets Mac sont signés ad hoc, sans notarisation Apple. Consultez le guide pour les prérequis et les limites des tests.

[Guide / English](../../cross-platform.md)

## 3.9.0 — 2026-11-01

### Gestion des torrents

Prise en charge des liens magnet du navigateur, du glisser-déposer de fichiers torrent et des suggestions du presse-papiers. Le partage après téléchargement et les connexions entrantes sont deux options distinctes, désactivées par défaut. Analyse de plusieurs fichiers, déplacement et renommage des dossiers ; Tout arrêter interrompt aussi le partage.

[Guide / English](../../user-guide.md#download-torrents)

## 3.8.0 — 2026-11-01

### Ouvrir des torrents depuis Windows

Ouvrez les fichiers .torrent depuis l’Explorateur et les liens magnet depuis le navigateur pour les vérifier. L’inscription facultative dans Windows préserve les applications par défaut et les éléments en attente après redémarrage. La sécurité des torrents a été renforcée : un magnet sans métadonnées en cache nécessite un tracker, sinon utilisez un fichier .torrent. Des tests de régression couvrent la protection existante contre les conflits de noms de fichiers.

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

## 3.3.3 — 2026-10-28

### Améliorations de l’interface

L’éditeur et les scripts chargés disposent désormais d’onglets distincts. Recherchez ou créez un script dans la liste, puis ouvrez l’éditeur complet depuis l’en-tête Lua ou PowerShell. Enregistrement, exécution/arrêt et journaux facultatifs restent près du code. Les commandes de fichiers sont regroupées dans le menu d’actions de l’éditeur. Le changement de vue conserve le code non enregistré, les réglages et la position du curseur.

### Des actions de téléchargement plus réactives

Les mises à jour des files et des états, les demandes vidéo du navigateur et les ajouts de téléchargements sont traités en arrière-plan pour éviter que le stockage lent ne bloque les clics. Les fenêtres affichent la progression, empêchent les envois en double et conservent les saisies en cas d’échec. Les dossiers s’ouvrent aussi en arrière-plan et les anciens résultats ne remplacent pas la nouvelle file sélectionnée.
