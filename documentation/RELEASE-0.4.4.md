# Iriscope 0.4.4

Cette version regroupe les corrections de fiabilité de la revue du 11 octobre,
ainsi que des optimisations de taille et de traitement des photos.

- Mises à jour : le paquet téléchargé reste cohérent avec la version affichée.
  Une nouvelle recherche ou un nouveau téléchargement écarte l'ancien paquet ;
  l'installation attend la fin des opérations en cours.
- Caméra : restauration cohérente des modes automatiques et des valeurs, avec
  protection contre les réponses tardives après une nouvelle modification ou
  une remise à zéro des réglages.
- Captures : récupération du journal après correction du nom d'un dossier, en
  conservant son identifiant. Les associations incohérentes restent refusées.
- Vidéos interrompues : récupération des images JPEG réellement décodables,
  avec contrôle de leurs dimensions avant la reconstruction de l'AVI.
- macOS : une commande de réglage expirée n'est plus appliquée tardivement.
- Photos : les réglages de couleur neutres évitent un parcours inutile des
  pixels ; le miroir travaille dans le même tampon, sans copie supplémentaire.
- Taille : les exécutables distribués retirent les symboles de compilation,
  tout en gardant les optimisations de vitesse et la gestion des erreurs des
  tâches de fond.
- Publication : le workflow vérifie le formatage, Clippy et les tests Rust/Python
  avant de publier les installateurs et leur manifeste signé.

Deux téléchargements : **Linux amd64 `.deb`** et **macOS Intel `.dmg`**.
Depuis la 0.4.3, la mise à jour est accessible dans **Paramètres**. Depuis une
0.4.1 ou 0.4.2, installer cette version une fois manuellement. L'installation
peut demander une authentification système. La signature Mac reste ad hoc,
sans notarisation.

Les nouveaux paquets sont construits sur Linux et Mac Intel. Les essais matériels
Mac Intel de la 0.4.3 sont documentés ; cette publication ne constitue pas un
nouvel essai physique de la 0.4.4. Windows et Apple Silicon restent en attente
de validation matérielle et ne sont pas distribués dans cette release.

Voir la [revue du code](https://github.com/Epikaigle/iriscope-app/blob/v0.4.4/documentation/REVUE-CODE-2026-10-11.md)
et les [mesures de performance](https://github.com/Epikaigle/iriscope-app/blob/v0.4.4/documentation/PERFORMANCE.md).
