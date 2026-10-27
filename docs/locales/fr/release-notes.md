## 3.1.1 — 2026-10-27

### Finalisation plus rapide des gros fichiers

Les téléchargements HTTP/HTTPS et FTP en une seule partie sur le même lecteur sont désormais vérifiés et enregistrés sans écrire une seconde copie complète. Les téléchargements en plusieurs parties conservent leur assemblage asynchrone dans le bon ordre. L’enregistrement sur un autre lecteur conserve la copie sécurisée. La vérification des sommes de contrôle, l’annulation, la protection des noms existants et la marque de provenance Internet sont maintenues. Un enregistrement interrompu est récupéré depuis la partie enregistrée ou le fichier d’assemblage, sans téléchargement supplémentaire ni doublon.
