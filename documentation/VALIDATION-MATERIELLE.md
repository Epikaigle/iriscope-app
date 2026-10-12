# Validation du DE400 avant diffusion

Ces essais doivent être exécutés avec la caméra réelle sur chaque système pris en charge. Les tests cloud, les images synthétiques et la compilation croisée ne remplacent pas cette validation.

État au **12 octobre 2026** : la version préparée est la 0.4.5, avec deux
installateurs, Linux amd64 `.deb` et macOS Intel `.dmg`. Le bouton, les captures
et les réglages du DE400 sont documentés sous Linux et sur Mac Intel sous
macOS 15.7.7. Les [essais Mac du 8 octobre](VALIDATION-MACOS-2026-10-08.md)
incluent quatre appuis physiques et les huit réglages ; le
[rapport 0.4.3 du 9 octobre](VALIDATION-0.4.3-MACOS-2026-10-09.md) vérifie aussi
le paquet et la mise à jour. Windows et Mac Apple Silicon ne sont pas publiés
dans cette release et restent sans validation matérielle. La liste suivante
est le protocole à compléter pour chaque système, pas une affirmation que
tous ses parcours ont déjà été exécutés.

La revue du 12 octobre ajoute trois essais Linux de 60 secondes d'aperçu et
60 secondes de vidéo, dont un avec une construction intermédiaire optimisée
avant la dernière correction du mode développement. Les captures, l'export
et la sauvegarde/restauration ont réussi. Ces essais ne comportent pas de
nouvelles pressions physiques et ne remplacent pas une session de 30 minutes.
Un quatrième essai avec le binaire final 0.4.5 a ensuite réussi : 60 secondes
d'aperçu, 60 secondes de vidéo (481 images lisibles), export MP4 et
sauvegarde/restauration identique. Les résultats et leurs limites figurent
dans [PERFORMANCE.md](PERFORMANCE.md).

## Préparation

Construire la version de distribution avec `cargo build --release -p iriscope-app --locked`. Utiliser un dossier de captures temporaire, deux dossiers patients fictifs et les deux côtés d’œil. Noter la version, le système, le modèle de caméra, le pilote, la résolution et l’échelle de l’écran. Ne pas utiliser de données de personnes réelles dans les rapports.

Lancer `iriscope-app --diagnose` et conserver le résultat localement. Vérifier les modes exposés par cette caméra : sous Linux, l’exemplaire DE400 diagnostiqué le 1er octobre propose 1280 × 1024 MJPEG à 8 FPS, sans mode 30 FPS à cette résolution. Le chemin USB direct Mac validé utilise 1280 × 1024 YUYV à 8 FPS.

## Parcours à valider

- Démarrage avec et sans caméra ; branchement après ouverture ; déconnexion/reconnexion répétée, y compris pendant une photo ou une vidéo.
- Au moins 30 minutes de direct en version release : relever la cadence mesurée, le temps de décodage, les images perdues et l’évolution de la mémoire. Répéter pendant un enregistrement vidéo.
- Capture répétée des deux yeux, changement puis fin de session. Vérifier dossier, œil, original enregistré et contexte affiché en plein écran.
- Plein écran, survol et disparition des commandes, focus clavier, Échap, zoom et déplacement. Déplacer la fenêtre entre deux écrans à des échelles différentes.
- Réglages natifs disponibles, modes automatiques/manuels, réinitialisation et persistance après redémarrage. Le chemin USB direct Mac expose les huit réglages mesurés sur le DE400 ; AVFoundation seul expose moins de contrôles.
- Recherche par dossier, dates inclusives, œil, ordre ancien/récent, pagination et navigation entre pages depuis la visionneuse.
- Correction d’une faute dans un dossier : conserver son numéro et les associations des captures anciennes et nouvelles.
- Comparaison de deux photos avec dates visibles, zoom et déplacement synchronisés ; effacement de la référence ; lecture et déplacement dans une vidéo.
- Sélecteurs natifs, chemins avec espaces/accents, annulation, volume en lecture seule et espace presque plein. Vérifier qu’une vidéo peut toujours être arrêtée.
- Sauvegarde sur un autre volume, restauration dans un nouveau dossier, sauvegarde altérée refusée, présence et récupération d’une vidéo `.part`. Vérifier que les captures actuelles restent intactes.
- Annuler une copie contenant une grande vidéo et vérifier le retrait du dossier temporaire ; contrôler la date de dernière sauvegarde et la persistance du rappel facultatif après redémarrage.
- Exporter une vidéo en MP4 avec FFmpeg, vérifier lecture, durée et original AVI intact ; annuler un export et vérifier qu’aucun MP4 partiel n’est publié. Vérifier aussi le message obtenu en l’absence du moteur d’export.
- Fermer l’application pendant une écriture ou une sauvegarde ; redémarrer et vérifier les médias publiés et la reprise des captures.
- Installer et désinstaller le paquet natif. Vérifier le raccourci, les autorisations caméra et la conservation des captures et des paramètres personnels.
- Mettre à jour une installation existante vers la version publiée : vérifier le numéro de version, les réglages caméra, les dossiers patients et les captures antérieures. Une 0.4.1 ou 0.4.2 nécessite d'abord une installation manuelle de la 0.4.3 ; les mises à jour suivantes utilisent les installateurs depuis Paramètres.

## Bouton physique Snapshot

Sous Linux, le signal a été identifié sur le DE400 `21cd:603b` le 6 octobre 2026 : trois pressions réelles ont produit chacune `02 01 00 01`, puis `02 01 00 00` au relâchement, sur l’endpoint d’interruption `0x81`. Le pilote ne crée pas de touche caméra car l’appareil déclare `bTriggerUsage = 0`. La [passerelle Linux](../helpers/linux/README.md) transmet ce signal au backend. Windows essaie un abonnement natif optionnel, à valider avec le pilote réel. Sur Mac Intel/macOS 15.7.7, l'ouverture USB ordinaire est refusée par le pilote Apple, mais le chemin USB direct avec service autorisé a réussi deux photos puis le démarrage et l'arrêt d'une vidéo par quatre appuis physiques. Apple Silicon reste à tester. Voir [l'état multiplateforme](BOUTON-MULTIPLATEFORME.md).

Sous Linux, après fermeture du direct, utiliser l’outil existant `python3 scripts/diagnose-de400-button.py --seconds 45 --video /dev/video0 --metadata /dev/video1` et effectuer trois pressions espacées. Adapter les chemins après identification du périphérique. L’outil observe les métadonnées UVC sans commandes USB propriétaires et sans enregistrer d’images.

Sur cet exemplaire, aucun marqueur STILL_IMAGE n’a été observé pendant les pressions. Pour observer le signal utilisé, lancer `sudo python3 scripts/diagnose-de400-usb-status.py`, puis appuyer trois fois après `STATUS_ARMED`. L’outil ne conserve pas d’images et ne modifie pas les contrôles USB.

Après installation de la passerelle et compilation, lancer `target/release/iriscope-app --validate-button target/validation-bouton` avec une destination vide. Deux pressions prennent les photos des deux yeux pour deux dossiers homonymes distincts ; les deux suivantes démarrent et arrêtent une vidéo. Suivre les messages du terminal. Le rapport vérifie aussi les noms, la lisibilité de chaque image vidéo, la finalisation, l’export et la sauvegarde/restauration. Tester séparément un appui prolongé, la déconnexion/reconnexion et les actions de bouton choisies dans les paramètres. Le scénario automatique est `--validate-hardware target/validation-auto`.

## Compte rendu

Le workflow [Release macOS Intel et Linux](../.github/workflows/release.yml)
construit les deux paquets sur un tag `v*` ou à la demande. Avant le paquet
Linux, il vérifie le formatage Rust, Clippy strict, les tests Rust du workspace
sous Xvfb et les tests Python. Il peut réutiliser un paquet Mac déjà testé.
Il n'y a plus de CI générale automatique sur les changements du logiciel ;
les changements du site dans `docs` sur `main` déclenchent sa publication.
Ces contrôles de code et de paquets ne constituent pas un essai physique du DE400.

Les commandes adaptées aux installations Windows et macOS sont dans
[Essayer sur Windows et macOS](ESSAIS-WINDOWS-MACOS.md). Le rapport indique le
système et l’architecture réellement testés. Si FFmpeg est absent, seul l’export
MP4 est marqué **NOT TESTED** ; les captures et leur sauvegarde/restauration sont
quand même vérifiées. Toute autre erreur d’export reste un échec.

Pour chaque parcours, noter « réussi », « échoué » ou « non testé », le comportement obtenu et les étapes de reproduction. Un échec ou un essai non réalisé bloque uniquement l’affirmation correspondant à ce parcours, et ne doit pas être remplacé par un résultat synthétique.
