# Iriscope 0.4.5

Cette version réduit le travail répété pendant l'aperçu, l'enregistrement et
les réglages des photos, en conservant la qualité et les contrôles des captures.

- **Caméra MJPEG** : aperçu et enregistrement réutilisent le décodage d'une
  même image. Chaque image enregistrée reste vérifiée ; une erreur ou des
  tailles hors limites ne sont pas acceptées pour accélérer l'écriture.
- **Affichage** : le JPEG est décodé directement dans son tampon d'affichage,
  en évitant une allocation RGB intermédiaire et une copie complète.
- **Photos** : luminosité et contraste utilisent une table de 256 valeurs.
  Rotation, correction et miroir alimentent directement le tampon final,
  avec les mêmes pixels qu'auparavant. Les réglages neutres réutilisent le
  tampon original sans copie.
- **Lecture vidéo** : en pause, le lecteur attend une action au lieu de se
  réveiller toutes les 20 ms. Reprise, déplacement dans la vidéo, fermeture
  et changement de fichier interrompent cette attente.
  Un remplacement de vidéo ne peut plus bloquer le worker ; la première image
  reste affichée pendant son intervalle, même à faible cadence.
- **Mesures** : de nouveaux benchmarks synthétiques et un outil de suivi
  CPU/mémoire sous Linux permettent de refaire les comparaisons avec un
  dossier de captures de test distinct.

Sur le PC Linux de la revue du 12 octobre 2026, les benchmarks montrent un
travail de décodage aperçu + validation vidéo ramené d'environ 90 à 44–45 ms
par image. Le réglage de luminosité/contraste passe de 14,156 à 3,364 ms ;
le parcours photo avec rotation 37° et miroir passe de 93,492 à 75,495 ms.
La rotation bilinéaire seule reste proche de 55 ms. Ces résultats portent
sur des données synthétiques et ne prédisent pas la latence du direct sur
toutes les machines. Cette série précède la dernière correction du mode
développement, qui isole aussi les génériques JPEG des dépendances non
optimisées. La LTO `thin`, qui agrandissait le binaire mesuré,
n'est pas activée.

Trois parcours avec le DE400 réel, dont une construction intermédiaire
optimisée, ont terminé sans erreur, avec 60 secondes
d'aperçu puis 60 secondes de vidéo chacun. La vidéo optimisée contient
483 images lisibles à une cadence nominale de 8 images/s. Photos, dossiers,
associations, export MP4 et sauvegarde/restauration ont été vérifiés ; l'AVI
original et les octets des médias restaurés sont conservés. Ces essais
n'incluent pas de nouvelles pressions sur le bouton physique. Un quatrième
parcours avec le binaire final 0.4.5 a aussi réussi, avec 481 images lisibles,
export MP4 et sauvegarde/restauration identique des cinq captures.

Validation locale : **291 tests Rust réussis, aucun échec**, Clippy strict
et **23 tests Python réussis**. Cinq tests restent ignorés dans l'exécution
courante : les trois benchmarks manuels, exécutés séparément pendant la
revue, et deux essais matériels DE400. La publication exige aussi la réussite
des vérifications de publication et des deux builds natifs des installateurs.
Les contrôles stricts de compilation passent également pour Windows x86-64,
Mac Intel et Apple Silicon ; ils ne remplacent pas les essais matériels.

Le partage concerne MJPEG ; les conversions YUYV brutes, notamment sur Mac,
ne sont pas encore mutualisées. Le décodage JPEG reste sur CPU. Aucun nouvel
essai matériel macOS ou Windows n'est revendiqué pour cette version, et les
essais de 60 secondes ne remplacent pas une validation de stabilité prolongée.
Windows et Apple Silicon restent en attente de validation matérielle pour
leur distribution.

Voir les [mesures et commandes reproductibles](https://github.com/Epikaigle/iriscope-app/blob/v0.4.5/documentation/PERFORMANCE.md), le
[protocole de validation matérielle](https://github.com/Epikaigle/iriscope-app/blob/v0.4.5/documentation/VALIDATION-MATERIELLE.md) et les
[informations sur le bouton selon le système](https://github.com/Epikaigle/iriscope-app/blob/v0.4.5/documentation/BOUTON-MULTIPLATEFORME.md).
