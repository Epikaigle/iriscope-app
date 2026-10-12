# Mesures reproductibles

Les commandes suivantes créent uniquement des données synthétiques temporaires ; elles ne lisent pas le dossier de captures de l’utilisateur.

```bash
cargo run --release -p iriscope-core --example benchmark_library --locked -- 1000
cargo run --release -p iriscope-core --example benchmark_library --locked -- 10000
cargo run --release -p iriscope-imaging --example benchmark_decode --locked
```

Le premier outil mesure le scan des métadonnées, la sélection d’une page filtrée et la vérification de 100 captures. Chaque fichier contient 64 Kio de données synthétiques ; cet outil ne décode pas les images. Le second décode 100 JPEG synthétiques de 1280 × 1024 pixels avec un contenu bruité pour solliciter le décodeur. Il inclut le coût d’allocation et affiche la médiane et le 95e percentile.

Mesures du 5 octobre 2026 dans l’environnement cloud Linux, en release :

| Charge | Scan des métadonnées | Sélection de 100 | Vérification de 100 |
|---|---:|---:|---:|
| 1 000 captures | 16,29 ms | 0,02 ms | 22,69 ms |
| 10 000 captures | 362,89 ms | 1,11 ms | 132,42 ms |

JPEG bruité de 2 070 799 octets : décodage médian 44,05 ms, 95e percentile 57,33 ms. Ces résultats dépendent du matériel, du stockage et des autres tâches en cours. Ils ne mesurent pas la latence du direct ni la cadence soutenue du DE400.

Pour ces dernières mesures, utiliser le binaire release et le protocole de [validation matérielle](VALIDATION-MATERIELLE.md). Observer la mémoire pendant 30 minutes, pendant l’enregistrement et après navigation répétée dans de grandes bibliothèques. Les budgets de queues et de caches sont déjà bornés ; toute hausse continue doit être analysée avant d’ajouter un autre cache.

Pour mesurer une vidéo longue, fournir un JPEG **synthétique** de 1280 × 1024 pixels, inférieur à 128 Kio :

```bash
cargo run --release -p iriscope-core --example benchmark_video --locked -- /chemin/image-synthetique.jpg
```

L’outil crée un AVI temporaire d’une heure à 8 FPS, contrôle ses 28 800 images et vérifie 100 positions de lecture grâce à un identifiant dans chaque image. Il vérifie l’espace libre avant de créer le fichier et supprime ses données temporaires après l’essai. Pour le JPEG synthétique de 99 302 octets utilisé ici, le fichier fait 2 860 821 256 octets : indexation 100,53 ms, lecture d’une image après déplacement médiane 0,04 ms et 95e percentile 0,15 ms. Ces temps couvrent le conteneur et les lectures, pas le décodage, le rendu ni un enregistrement matériel en temps réel.

## Taille et ressources — revue du 11 octobre 2026

La 0.4.4 utilise `opt-level = 3` et `strip = "symbols"` en release.
Ces options suivent les [profils documentés par Cargo](https://doc.rust-lang.org/cargo/reference/profiles.html).
La suppression des symboles réduit le fichier distribué ; elle ne supprime pas
les fonctionnalités et ne constitue pas un gain mesuré de mémoire ou de CPU.
Le déroulement de pile reste activé, car les tâches de fond utilisent
`catch_unwind` pour traiter certaines erreurs. Les exécutables de développement
conservent leurs symboles.

Mesure isolée sur une copie des exécutables extraits du Debian **0.4.3 publié**,
avant et après `strip --strip-all` :

| Exécutable Linux | Avant (octets) | Après (octets) | Réduction |
|---|---:|---:|---:|
| Application | 50 343 328 | 37 662 600 | 25,19 % |
| Composant de mise à jour | 1 179 104 | 967 480 | 17,95 % |

Cette comparaison porte sur le même code de la 0.4.3, pour isoler les symboles.
Elle ne prédit pas exactement la taille du nouveau paquet compressé 0.4.4.
Les deux copies affichent toujours leur version correcte après suppression
des symboles ; les nouveaux paquets passent leurs propres contrôles.

Deux parcours évitables ont également été retirés du traitement des photos :
une luminosité et un contraste neutres évitent la boucle de correction RGB ;
le miroir modifie le tampon existant, sans allouer une seconde image complète.
Un test compare les pixels au traitement précédent sur 40 combinaisons de
réglages, angles et miroir, avec toutes les valeurs possibles des canaux.

### CPU, GPU et organisation

- Le direct, les écritures photo/vidéo, la bibliothèque et la visionneuse
  utilisent des tâches distinctes. Elles peuvent travailler sur plusieurs cœurs
  sans bloquer le fil graphique ; les files d'attente et caches sont bornés.
- Le décodeur JPEG utilise les accélérations SIMD disponibles (x86/ARM).
  Le direct conserve la dernière image et suspend le décodage lorsqu'il n'est
  pas visible. Les miniatures sont demandées pour les éléments affichés.
- Slint dispose du rendu GPU FemtoVG et d'un rendu logiciel de secours.
  Le décodage JPEG et les transformations des photos exportées restent sur CPU.
  L'application ne dispose pas d'un décodeur JPEG GPU dédié.
- Les sept crates séparent interface, logique, imagerie, mises à jour et
  systèmes de caméra. Déplacer ou fusionner des fichiers ne rend pas à lui seul
  l'exécutable plus rapide. Le dossier `target` est un cache de compilation
  local et n'est pas inclus dans les installateurs.

## Optimisations de la 0.4.5 — mesures du 12 octobre 2026

Les comparaisons suivantes ont été faites sous Linux sur un AMD Ryzen 5 2600
(6 cœurs, 12 fils), avec les optimisations release. Elles distinguent les
traitements synthétiques isolés du parcours avec le DE400 physique.

### Traitements et buffers

- L'aperçu et l'enregistrement partagent le résultat du décodage d'une même
  image MJPEG. L'ordre d'arrivée des deux consommateurs ne change pas ce
  résultat. Chaque image enregistrée reste validée : les erreurs de décodage
  et les tailles hors limites restent refusées.
- Le JPEG de l'aperçu est décodé directement dans son tampon Slint. À
  1280 × 1024, cela évite un tampon RGB intermédiaire de **3 932 160 octets**
  et sa copie. Ce volume évité n'est pas une baisse garantie de la mémoire
  résidente du processus, dont l'allocateur et les autres tâches varient.
- Luminosité et contraste utilisent une table de 256 valeurs. En présence
  d'une rotation, la correction est appliquée aux pixels source avant
  l'interpolation, comme auparavant. Le rendu écrit directement dans son
  tampon final et applique le miroir dans les coordonnées de destination.
  Les réglages neutres partagent le tampon original sans recopier les pixels.
- La lecture vidéo en pause attend un changement de lecture, de position ou
  de fichier. Elle ne se réveille plus périodiquement toutes les 20 ms.
  Pendant la lecture, l'attente respecte l'échéance de l'image suivante et
  reste interruptible par les commandes de la visionneuse.

Mesures synthétiques sur des images de 1280 × 1024 pendant la revue :

| Traitement mesuré | Avant | Après | Périmètre |
|---|---:|---:|---|
| Décodage MJPEG pour aperçu + validation vidéo | 90,14 ms/image | 44,06 / 44,83 ms/image | Médiane de trois séries de 40 images ; aperçu en premier / enregistrement en premier |
| Décodage JPEG vers l'affichage | 38,489 ms | 36,135 ms | Médiane de 100 images, copie comprise dans l'ancien chemin |
| Affichage avec luminosité + contraste | 14,156 ms | 3,364 ms | Médiane de 40 essais, allocations et copies comprises |
| Affichage avec luminosité + contraste + rotation 37° + miroir | 93,492 ms | 75,495 ms | Médiane de 40 essais, mêmes pixels de sortie |
| Changements de contexte volontaires du fil en pause | 60 | 1 | Deux attentes isolées de 1,2 seconde sous Linux |

Le benchmark aperçu + vidéo utilise un JPEG bruité de 3 167 679 octets ; celui
du décodage vers Slint utilise un JPEG de 2 070 799 octets. La comparaison des
photos alterne les chemins après cinq essais de chauffe. Elle isole les
anciennes copies et la formule par canal ; les deux chemins utilisent le
même moteur de rotation. Ces temps ne couvrent pas l'affichage à l'écran,
le transfert vers le GPU, les écritures vidéo ou la latence du bouton.
Cette série précède la dernière isolation des génériques JPEG en mode
développement ; elle décrit une construction intermédiaire de la 0.4.5.

En développement, le décodeur utilise désormais un lecteur privé compilé
dans `iriscope-imaging`, pour éviter de partager une implémentation générique
non optimisée avec la dépendance `image`. Sans modifier les profils, le
benchmark d'affichage direct passe de 382,600 à 113,978 ms sur le même JPEG
bruité. Le décodage RGB seul prend 38,43 ms ; l'allocation et la conversion
des tampons d'affichage restent plus lentes en développement. Ces mesures
ne doivent pas être confondues avec celles de la distribution release.
Sur le binaire final, une nouvelle série release donne **52,844 ms avec
copie et 49,109 ms en direct** (95e percentiles : 79,537 et 91,068 ms).
La copie évitée améliore ici la médiane, sans amélioration démontrée de la
latence au 95e percentile. La charge des autres applications reste variable.

La rotation seule à 37° reste proche de 55 ms : **55,049 ms avant et
55,395 ms après**. Aucun gain significatif n'est revendiqué pour son seul
calcul bilinéaire. Les dimensions et les sommes de contrôle sont identiques
pour les angles 0°, 37°, 90°, 135°, 180°, 270° et 315°. Le gain du parcours
photo vient de la table de correction et des copies et passages évités.

Pour reproduire ces mesures sans lire les captures de l'utilisateur :

```bash
cargo run --release -p iriscope-app --example benchmark_display_decode --locked
cargo run --release -p iriscope-imaging --example benchmark_rotation --locked
cargo test --release -p iriscope-app --lib shared_camera_benchmark --locked -- --ignored --nocapture --test-threads=1
cargo test --release -p iriscope-app --lib benchmark_photo_display --locked -- --ignored --nocapture --test-threads=1
cargo test --release -p iriscope-app --lib benchmark_paused_playback_waits --locked -- --ignored --nocapture --test-threads=1
```

Le dernier benchmark lit les changements de contexte dans `/proc` et est
réservé à Linux. Les benchmarks manuels sont ignorés dans les tests courants,
puis exécutés explicitement pour les mesures. Celui de la rotation seule
inclut l'allocation de sortie mais exclut le clone d'entrée du chronomètre.

### Parcours avec le DE400 physique

Trois parcours ont utilisé le binaire de référence, puis une construction
intermédiaire optimisée (avant la dernière correction du mode développement),
puis à nouveau la référence comme contrôle, avec **60 secondes d'aperçu et
60 secondes d'enregistrement par parcours**.
Le script échantillonne le CPU et la mémoire du processus une fois par seconde
et exclut les trois premières secondes de chaque phase :

| CPU moyen du processus | Référence initiale | Optimisé | Référence de contrôle |
|---|---:|---:|---:|
| Aperçu | 42,72 % | 34,53 % | 48,96 % |
| Aperçu + enregistrement | 75,67 % | 48,78 % | 72,76 % |

Ces pourcentages sont relatifs à **un cœur**, et non à l'ensemble du CPU.
La baisse est observée dans ces parcours ; elle n'est pas une garantie de
gain sur chaque ordinateur. D'autres applications et des compilations
tournaient pendant la comparaison, et la scène devant la caméra pouvait
varier. Ce protocole n'isole pas le seul effet de chaque optimisation.
Il ne prouve ni une stabilité pendant 30 minutes ni une consommation
électrique ou une latence GPU améliorée.

Les trois parcours ont terminé sans erreur. La vidéo optimisée contient
**483 images JPEG lisibles**, à une cadence nominale de 8 images/s. Le
parcours vérifie les photos, les deux yeux, les dossiers homonymes avec des
identifiants distincts, les associations dans la bibliothèque et l'export
MP4. L'export conserve l'AVI original ; sauvegarde et restauration conservent
exactement les octets des médias et leurs associations. Ces essais utilisent
les actions de capture de l'application : ils ne comprennent pas de nouvelles
pressions sur le bouton physique.

Un quatrième parcours avec le binaire final 0.4.5 a également réussi, avec
60 secondes pour chaque phase et **481 images JPEG lisibles** dans la vidéo.
CPU moyen : **14,48 % en aperçu et 15,70 % en aperçu + enregistrement**.
Mémoire résidente : 169 156–176 940 Kio en aperçu et 194 952–199 320 Kio
pendant la vidéo. Aucun abandon d'aperçu n'est relevé dans les échantillons.
L'appareil avait été rebranché et la scène et la charge pouvaient différer
des trois premiers essais : ces valeurs ne permettent pas d'attribuer une
réduction supplémentaire au seul correctif du décodeur.
SHA-256 du binaire local testé :
`443b465d2cb648be750d8f3cfd0f6a55be3b193e4c1dbd7f788c8dc6df5f2aef`.

Pour refaire un essai, fermer l'autre instance d'Iriscope et réserver un
**nouveau dossier vide** ; le script conserve les captures de test et leurs
rapports à côté de ce dossier :

```bash
cargo build --release -p iriscope-app --locked
python3 scripts/profile-camera.py target/release/iriscope-app /chemin/essai-iriscope-neuf --seconds 60
```

`--seconds 1800` permet un suivi de 30 minutes **pour chaque phase**. Le
protocole complet reste décrit dans la [validation matérielle](VALIDATION-MATERIELLE.md).
Les mesures et captures locales de cette revue sont conservées dans
`target/performance-review`, qui n'est pas distribué dans les installateurs.

### Comparaison du compilateur

La LTO `thin` a été essayée sur le même code et avec suppression des symboles,
sans modifier le profil retenu pour la distribution :

| Exécutable Linux | Profil habituel | LTO `thin` |
|---|---:|---:|
| Taille brute | 37 691 272 octets | 41 799 560 octets |
| Compression `xz -6` | 8 252 480 octets | 9 067 312 octets |

La LTO agrandit ici le binaire et sa copie compressée ; elle est donc
**écartée**. Ces tailles concernent l'exécutable mesuré, pas un installateur
0.4.5 publié. Le profil conserve `opt-level = 3`, `strip = "symbols"` et le
déroulement de pile pour les erreurs des tâches de fond.

### Qualité conservée et limites

Les tests comparent les pixels au traitement précédent pour chaque angle
entier, avec miroir, tables de correction, images non carrées et dimensions
limites. Ils contrôlent aussi le décodage JPEG direct, les buffers invalides,
la concurrence des consommateurs et les commandes de lecture en pause.
La revue locale comprend 291 tests Rust réussis, aucun échec et cinq tests
ignorés dans l'exécution courante (trois benchmarks manuels et deux essais
DE400). Les trois benchmarks ci-dessus ont ensuite été exécutés séparément.
Clippy strict et les 23 tests Python ont également réussi.
Après la dernière correction du lecteur JPEG, les 23 tests d'imagerie et
Clippy strict ont été relancés. Les contrôles stricts de tous les targets du
workspace passent aussi pour Windows x86-64, Mac Intel et Apple Silicon.

Le partage du décodage concerne MJPEG. Il ne mutualise pas encore la
conversion des images brutes YUYV, notamment utilisées sur macOS. Il n'y a
pas de décodage JPEG GPU dédié et ces mesures ne constituent pas un nouvel
essai matériel sur Mac ou Windows. La validation prolongée et les essais
avec l'appareil sur ces systèmes restent nécessaires.

Il n'est pas démontré que l'application est optimisée au maximum. Utiliser
tous les cœurs ou le GPU à 100 % n'est pas un objectif : les critères utiles
sont la latence, la cadence, la mémoire et la consommation d'énergie. Toute
nouvelle optimisation doit conserver les pixels et l'intégrité des captures,
puis montrer un gain mesuré dans le parcours concerné.
