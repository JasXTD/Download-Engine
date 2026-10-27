## 3.1.0 — 2026-10-27

### Vidéos plus rapides et charge de transfert réduite

Les vidéos téléchargent jusqu’à 4 segments en parallèle en respectant les limites globales ou par site plus basses. La pause, les nouvelles tentatives et le redémarrage conservent tous les segments complets vérifiés, même terminés dans le désordre ; les segments incomplets sont retéléchargés. La fenêtre d’état affiche les segments et connexions actifs. Les écritures en tampon, les mises à jour de progression moins fréquentes et la réutilisation des connexions à la base réduisent la charge. La pause attend toujours la fin des écritures et chaque bloc reçu reste comptabilisé durablement dans le quota.

### État d’échec plus clair

L’état d’erreur devient « Échec » ; les détails et tentatives automatiques restent inchangés.
