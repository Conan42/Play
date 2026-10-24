# Architecture de Play

Play reprend le découpage visible de `Exec_modernise` V1 sans reprendre son
noyau RISC-V, son ABI U-Mode ou son format ExecFS. Les contrats portables sont
séparés de l'application desktop afin que le modèle et les scènes puissent
être testés sans ouvrir de fenêtre.

## Vue des composants

```mermaid
flowchart LR
    Core[play-core\nmodèle et transport] --> Desktop[play-desktop]
    Media[play-media\nimages et analyse audio] --> Desktop
    UI[play-ui\nscènes et panneaux] --> Desktop
    Desktop --> Egui[egui / egui-winit / egui-wgpu]
    Desktop --> Native[winit, rfd, rodio, lofty]
    Desktop --> Window[fenêtre Linux / Windows / macOS]
```

`play-desktop` est le point d'assemblage. Il possède la boucle d'événements
`winit`, l'état `egui-winit`, le renderer `egui-wgpu`, la fenêtre de sélection
de fichiers et les interactions de la toolbar.

Le binaire reste volontairement réduit à `main.rs`, qui délègue le lancement à
`app::run`. Le code de l'application est organisé dans `apps/play-desktop/src/app/`
comme dans TacticalGlobe :

- `mod.rs` contient l'état partagé, les types de session et les primitives audio
  communes ;
- `config.rs` contient le format `playlist.ron`, la restauration de session et
  les réglages persistants, dont le cache des BPM ;
- `bpm.rs` contient la lecture des métadonnées audio, l'analyse BPM en arrière-
  plan et l'enregistrement des résultats ;
- `core.rs` contient l'initialisation de la fenêtre, la boucle `winit` et le
  cycle de rendu ;
- `player.rs` contient la liste musicale, les métadonnées, la lecture, les
  transitions et le bruit rose ;
- `room_analyzer.rs` contient la capture microphone et l'analyse de salle ;
- `ripper.rs` contient l'enregistrement des entrées système monitor en WAV et
  la vue dédiée du Ripper ;
- `live_tablature.rs` contient la détection de notes, la portée et la tablature ;
- `backing_track.rs` contient les stems et le mixage par piste ;
- `ui.rs` contient la toolbar, les mètres, les faders et l'assemblage des vues.

Chaque fichier complète `PlayDesktop` dans son domaine avec un bloc `impl`
séparé. Les modules utilisent les types partagés de `app::mod` sans mélanger
les contrôles spécifiques des cinq modes.

## Bibliothèques

### `play-core`

Cette crate est `no_std` avec `alloc`. Elle définit les contrats partagés :

- `WorkspaceMode` : `Player`, `RoomAnalyzer`, `LiveTablature`, `BackingTrack` et
  `Ripper` ;
- `TransportState` : `Stopped`, `Playing` et `Paused` ;
- `Track` et `ProjectDocument` pour la session et la liste de pistes ;
- `SpectrumSnapshot` pour les niveaux et la crête ;
- `Workspace` pour regrouper document, transport et spectre.

Elle ne dépend ni d'`egui`, ni de `wgpu`, ni d'une API audio système.

### `play-media`

Cette crate contient le traitement média indépendant de l'interface. Elle
fournit actuellement :

- `inspect_image`, qui valide une image et expose ses dimensions RGBA ;
- `SpectrumAnalyzer`, qui calcule 64 niveaux d'amplitude stéréo sur des
  portions temporelles du bloc audio, comme l'analyseur du Player d'Exec
  Modernisé ;
- `analyze_with_band_count`, qui permet au Room Analyzer d'utiliser 256 bandes
  mono avec une résolution fréquentielle supérieure ;
- `estimate_bpm_from_samples`, qui construit un flux spectral multibande sur
  les attaques audio, puis estime le tempo par corrélation temporelle en
  corrigeant les ambiguïtés moitié/double tempo ;
- `detect_pitch`, qui estime la fréquence fondamentale, la note MIDI et les
  cents ;
- `detect_chord`, qui estime la fondamentale et la qualité d'un accord à partir
  de l'énergie chromatique.

Le sample rate et le nombre de canaux sont récupérés dans la configuration de
l'entrée ou du fichier audio. Les flux microphone et les décodeurs sont
assemblés dans `play-desktop` autour des buffers d'analyse.

L'analyse BPM convertit jusqu'aux 120 premières secondes en mono, calcule une
FFT par fenêtres chevauchées et pondère les attaques dans les graves, les
médiums et les aigus. La corrélation retient un tempo dominant et normalise
les candidats moitié/double temps. Ce résultat reste une référence unique :
les changements de tempo et les interprétations half-time/double-time ne sont
pas encore exposés comme une carte par section dans la liste.

### `play-ui`

Cette crate est également `no_std` avec `alloc`. Elle définit `Scene` et
`PanelId`, donc le mode sélectionné et les panneaux visibles, sans contenir le
code de rendu `egui`.

## Composition de la fenêtre

```mermaid
flowchart TB
    Toolbar[Barre d'outils 38 px]
    ModeToolbar[Barre d'outils basse contextuelle]
    Main[Zone centrale]
    Tracks[Liste Pistes\nhauteur flexible et défilement]
    Info[Informations piste\net jaquette]
    Spectrum[Analyseur spectral stéréo\n64 bandes superposées, ancré en bas]
    Stereo[Mètres LED stéréo\ngraduation dB et faders L/R]
    Toolbar --> Main
    ModeToolbar --> Main
    Main --> Tracks
    Tracks --> Info
    Tracks --> Spectrum
    Main --- Stereo
```

La toolbar utilise les textures `assets/texture/bouton_normal.png` et
`assets/texture/bouton_enfoncer.png`, chargées par `egui` avec un filtrage
`NEAREST`. L'icône Material Icons est peinte par-dessus la texture et sa
couleur passe au jaune au survol ou lorsque la commande est active.

La zone centrale calcule la hauteur de la liste après le titre `Pistes`. Elle
réserve ensuite l'espace de la zone d'informations, du spectre et des
espacements. Ainsi, le cadre de la liste s'étend ou se réduit avec la fenêtre
tandis que les informations restent juste sous la liste et le spectre dans la
partie basse. Le panneau droit conserve toute la hauteur disponible pour les
mètres LED et les faders stéréo.

`RoomAnalyzer`, `LiveTablature`, `BackingTrack` et `Ripper` ajoutent chacun cette barre
d'outils contextuelle supérieure avec les mêmes boutons texturés. Elle expose les actions propres au mode
courant et disparaît pour `Player`, qui utilise directement la barre principale
et la liste du lecteur.

`BackingTrack` démarre avec une liste de stems vide. Les stems ajoutés depuis
le bouton dossier possèdent un nom, un rôle, un fichier, un volume, un
panoramique, un état mute et un état solo. Les réglages de mixage sont affichés
dans chaque ligne afin de laisser le panneau droit aux mètres des autres modes.
La liste des stems possède des défilements horizontal et vertical pour garder
les contrôles accessibles lorsque la fenêtre est plus étroite que la tranche
de mixage. La synchronisation des stems s'exécute sur un thread séparé et
publie sa progression à l'interface avant d'appliquer les décalages calculés.

Le mode `Ripper` capture le sink PipeWire par défaut avec `pw-record` sous
Linux ; sur les autres plateformes, il réutilise l'énumération des entrées
`rodio::microphone`. `ripper.rs` écrit les échantillons reçus dans un fichier
WAV PCM 24 bits sur un thread séparé et alimente le `LiveAnalysisBuffer` du
graphe stéréo. Le thread est arrêté et le fichier finalisé par le bouton d'arrêt
ou à la fermeture de l'application.

## Room Analyzer

Voir aussi [`ROOM_ANALYZER.md`](ROOM_ANALYZER.md) pour le détail de la capture,
de la résolution fréquentielle et du flux de traitement.

Le Room Analyzer ouvre un flux d'entrée avec `rodio::microphone` et le place
dans un buffer partagé avec la boucle d'interface. À chaque image, le desktop
extrait la fenêtre audio la plus récente, la convertit en mono et la transmet à
`SpectrumAnalyzer::analyze_with_band_count` avec 256 bandes.

Les niveaux sont affichés positivement sur une échelle de 0 à 110 dB, de 16 Hz
à 23 kHz, et le graphe occupe toute la zone centrale du mode. Le choix de
l'entrée et les erreurs d'ouverture sont affichés au-dessus du graphe.

La barre d'outils contextuelle permet de choisir le profil du microphone. `play-desktop` applique
la courbe nominale ECM8000 aux bandes du Room Analyzer lorsqu'elle est active,
avec une interpolation logarithmique ; une calibration individuelle reste la
référence pour les mesures précises.

Le signal de test est le fichier stéréo `assets/son/bruit_rose.wav`, chargé
comme source audio en boucle lorsque le mode Room Analyzer est actif et qu'aucune
piste ne joue. Le transport de la piste est conservé pendant le changement de
mode ; un bandeau affiche la piste active dans Room Analyzer et Live Tablature.

## Live Tablature

Voir aussi [`LIVE_TABLATURE.md`](LIVE_TABLATURE.md) pour les trois modes
d'affichage, la détection des notes et la synchronisation des mesures.

Le Live Tablature réutilise le même flux d'entrée et la même fenêtre d'analyse.
La détection de hauteur produit une note MIDI, puis `guitar_position` choisit
la position la plus grave disponible sur les six cordes dans les vingt
premières cases. Une détection d'accord est affichée dans l'en-tête.

Les positions reconnues sont ajoutées à une file temporelle limitée. Le
sélecteur d'affichage propose une tablature seule, une portée classique seule
ou les deux. La tablature est dessinée avec six lignes (`e B G D A E`) et la
portée classique avec cinq lignes et une clé de sol. Les deux systèmes
partagent les mêmes positions de notes et les mêmes barres de mesure. La durée
visible d'un système dépend de sa largeur, ce qui permet d'afficher davantage
de notes lorsque la fenêtre est agrandie.
La commande `Play/Pause` de la barre principale contrôle la piste audio.
`Play/Pause` de la barre d'outils contextuelle suspend ou reprend le défilement et l'ajout de notes, tandis que
`Enregistrer les notes` contrôle séparément la capture des positions détectées.
Les contrôles pédagogiques comprennent les onglets `TABLATURE` et
`STRUMMING`, le choix de mode, le choix d'affichage, le BPM, la mesure, le
métronome, les exercices et l'effacement de la tablature. L'horloge des
mesures démarre avec la première note capturée et est suspendue avec le
défilement.

Le flux actuel est audio. La connexion MIDI directe, la lecture d'un exercice
enregistré et la génération sonore du métronome sont des extensions à prévoir.

```mermaid
sequenceDiagram
    actor Guitariste
    participant Input as Microphone / entrée audio
    participant Buffer as LiveAnalysisBuffer
    participant Media as play-media
    participant Tab as LiveTablature

    Guitariste->>Input: joue une note ou un accord
    Input->>Buffer: pousse les échantillons
    Buffer->>Media: extrait la fenêtre récente
    Media->>Tab: fréquence, note, position et accord
    Tab->>Tab: ajoute la note et fait défiler la portée
```

## Flux de chargement des musiques

```mermaid
sequenceDiagram
    actor User
    participant Toolbar
    participant Dialog as rfd
    participant Core as play-core
    participant List as Liste des pistes

    User->>Toolbar: clique sur le bouton dossier
    Toolbar->>Dialog: ouvre la sélection native
    User->>Dialog: sélectionne un ou plusieurs fichiers
    Dialog->>Core: crée les Track
    Core->>List: met à jour la session
```

Les fichiers sélectionnés sont mémorisés dans l'état desktop pour éviter les
doublons. Leur nom sans extension devient le titre de piste et l'artiste
affiche `Fichier local`. Un clic sur une piste démarre son décodage et sa
lecture avec `rodio`; le bouton Lecture reprend la piste active ou démarre la
première piste chargée. La piste active est surlignée dans la liste. Lorsqu'un
FLAC contient une image intégrée, `lofty` la lit et le desktop l'affiche dans
la zone d'informations située sous la liste.

La configuration persistante est écrite dans `playlist.ron`. Elle contient une
entrée `bpm_cache` associée au chemin de chaque fichier. Au démarrage, le
lecteur restaure ce BPM pendant le chargement des métadonnées et évite ainsi de
relire les échantillons pour les fichiers déjà analysés. Le cache mémorise aussi
les analyses sans résultat ; une nouvelle analyse est lancée uniquement pour
une piste qui n'a pas encore été analysée.

## Vérification

Les commandes de validation sont :

```bash
cargo check --workspace
cargo test --workspace
cargo fmt --all -- --check
```

Les tests actuels vérifient la transition du transport, les niveaux
d'amplitude, la résolution des basses fréquences, la détection de hauteur et
la détection d'accord. La prochaine couche de tests devra couvrir le décodage,
l'audio réel, la file temporelle de tablature et la persistance des sessions.

## Évolutions prévues

1. ajouter la connexion MIDI directe et la lecture d'exercices enregistrés ;
2. valider les chemins de packaging des trois plateformes.
