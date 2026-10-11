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

Il n'est pas démontré que l'application est optimisée au maximum. Les copies
RGB vers Slint, la validation JPEG pendant l'enregistrement et les rotations
arbitraires sont des pistes à profiler en charge réelle. La LTO ou un autre
réglage du compilateur demanderaient également une comparaison mesurée.
Utiliser tous les cœurs ou le GPU à 100 % n'est pas un objectif : les critères
utiles sont la latence, la cadence, la mémoire et la consommation d'énergie.
