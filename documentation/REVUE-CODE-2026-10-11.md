# Revue du logiciel — 11 octobre 2026

Le dépôt local a été synchronisé avec `main` (`a5fb1a0`) sans écraser de travail
local. La revue couvre les captures et leur index, les réglages caméra, la
récupération vidéo, les mises à jour et les contrôles de publication. Les
correctifs ci-dessous sont des changements de source destinés à la prochaine
version ; ils ne remplacent pas les installateurs déjà publiés de la 0.4.3.

## Défauts corrigés

- **Mises à jour** : une vérification réussie invalide le paquet téléchargé
  précédemment avant de présenter la release trouvée. Un nouveau téléchargement
  retire aussi l'ancien paquet ; Installer refuse une opération encore active.
  Cela évite d'installer une version différente de celle affichée. Une erreur
  de vérification conserve la version connue et son paquet cohérent.
- **Réglages caméra** : les modes sauvegardés sont réappliqués après les valeurs
  numériques, qui peuvent forcer le mode Manuel sous Windows. La relecture
  actualise aussi le contrôle Auto associé. Les révisions des demandes protègent
  les nouvelles valeurs contre une réponse ancienne, y compris après un reset
  ou une resélection explicite du mode Auto déjà affiché.
- **Journal de capture** : une correction du nom d'un dossier conserve son ID
  et ne bloque plus la reprise d'une capture dont l'écriture de l'index avait
  échoué. La reprise utilise les noms actuels ; un ID disparu, un conflit
  d'association ou un média remplacé restent refusés. Il n'y a pas de
  réassociation automatique par nom.
- **Vidéo interrompue** : la récupération de l'application décode chaque JPEG
  et vérifie ses dimensions avant de publier l'AVI. Elle écarte les images
  illisibles et conserve la source si aucune image n'est récupérable. Le writer
  respecte désormais la limite d'un million d'images du lecteur, avant écriture.
  Après une récupération réussie, le parcours existant tente de retirer la source
  `.part` ; un échec de nettoyage est journalisé.
- **AVFoundation** : une commande de contrôle expirée dans la file est rejetée
  avant tout appel natif. Ce changement concerne Get/Set/Reset sur le chemin
  AVFoundation ; le composant USB direct du paquet Mac reste inchangé. Une
  opération déjà engagée dans le pilote natif ne peut pas être annulée ainsi.

Chaque défaut de logique est couvert par un test de régression. Les scénarios
caméra utilisent des doubles reproduisant les effets du pilote ; la vidéo
utilise de vrais JPEG, dont une table de quantification corrompue. Les tests de
journal provoquent un véritable échec d'écriture de l'index puis une correction
du dossier avant reprise.

## Publication et documentation

Le workflow existant de release exécute désormais le formatage, Clippy strict,
les tests Rust sous Xvfb et les tests Python sur Linux avant publication. Aucun
workflow supplémentaire à chaque changement du logiciel n'est ajouté. Les
guides matériels distinguent les essais réalisés des contrôles de compilation.
Les helpers d'installation Unix sont réservés à leurs systèmes ; Windows
compile sans avertissement et conserve l'indisponibilité explicite des mises
à jour intégrées. Une tentative de synchronisation de processus sur un système
non pris en charge échoue immédiatement plutôt que d'attendre inutilement.

La [release publique 0.4.3](https://github.com/Epikaigle/iriscope-app/releases/tag/v0.4.3)
contient exactement le `.deb` Linux amd64 et le `.dmg` Mac Intel. Le manifeste
public passe la vérification Ed25519 avec la clé du dépôt ; les tailles et
empreintes qu'il annonce correspondent aux métadonnées des deux assets GitHub.
Aucun installateur n'a été exécuté pour cette vérification du 11 octobre.

## Vérifications locales

- `cargo fmt --all --check` : réussi.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` : réussi.
- `SLINT_BACKEND=winit-software xvfb-run -a cargo test --workspace --locked` :
  **264 tests réussis**, aucun échec ; les deux tests DE400 réel sont ignorés
  par défaut. Le périphérique `21cd:603b` n'était pas détecté pendant cette revue.
- `python3 -m unittest discover -s tests -p 'test_*.py' -v` : **20 tests réussis**.
- Après le nettoyage de portabilité, les **10 tests de l'updater** sont relancés
  et réussissent ; ils sont déjà inclus dans le total des 264 tests.
- Clippy strict natif ciblé sur l'updater après ce nettoyage : réussi.
- `actionlint .github/workflows/release.yml` et `git diff --check` : réussis.

- Clippy strict sur **tout le workspace et toutes ses cibles**, avec `--locked`
  et `--target` : réussi pour Windows x86_64 MSVC, Mac Intel et Mac Apple Silicon.
  Il s'agit de contrôles de compilation, sans exécution des tests natifs de ces
  systèmes ni production d'installateurs depuis Linux.

Les journaux de travail restent dans `target/review-2026-10-11/`, hors Git. Cargo
utilise un seul processus de compilation à la fois pour limiter la mémoire.

## Limites et essais restants

Les rapports des [8 octobre](VALIDATION-MACOS-2026-10-08.md) et
[9 octobre](VALIDATION-0.4.3-MACOS-2026-10-09.md) documentent le bouton physique,
les captures, les réglages et une mise à jour sur Mac Intel/macOS 15.7.7. Cette
revue sur Linux ne constitue pas un nouvel essai physique sur Mac. Windows et
Apple Silicon restent sans validation matérielle et ne sont pas publiés.
L'installation/mise à jour Debian avec authentification Polkit réelle reste
également à vérifier ; les tests des scripts ne remplacent pas cet essai.

Les files photo/vidéo, le décodage et le cache de miniatures ont déjà des bornes,
et la bibliothèque est paginée. La revue n'a pas identifié de problème concret
justifiant une refonte de ces chemins. Aucune nouvelle mesure de cadence ou de
mémoire n'est déduite des tests de logique. Les modes Auto/Manuel et le reset
doivent être essayés sur la caméra réelle dans les prochains paquets.
