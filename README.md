# Play

Play est le playground multiplateforme issu des idées et des applications de
`Exec_modernise` V1. Le projet vise Linux, Windows et macOS avec un modèle
partagé en Rust et une interface native de bureau portée directement par
`egui`.

Le workspace est organisé en applications (`apps/`) et bibliothèques
partagées (`crates/`). L'interface desktop assemble ces bibliothèques sans
faire entrer `egui`, `winit` ou les API audio système dans les contrats
portables.

## État actuel

Le binaire `play` ouvre une fenêtre native `winit` rendue par `egui-wgpu`. La
session démarre avec une liste de pistes vide et propose :

- une barre d'outils compacte inspirée de TacticalGlobe, avec ses boutons
  38×38 et ses textures normale/enfoncée dans `assets/texture/` ;
- les commandes menu, précédent, lecture/pause, arrêt, suivant, répétition,
  enchaînement et chargement de fichiers ;
- le choix entre les modes `Player`, `RoomAnalyzer` et `UiLab` ;
- une fenêtre de chargement permettant de sélectionner plusieurs musiques
  (`mp3`, `flac`, `wav`, `ogg`, `m4a`, `aac` et `opus`) ;
- une liste de pistes qui occupe l'espace disponible et défile lorsqu'elle
  devient trop longue, avec une sélection visuelle de la piste active ;
- une zone d'informations après la liste, avec la jaquette intégrée aux FLAC ;
- un analyseur spectral stéréo ancré dans la partie basse de la fenêtre, avec
  64 bandes d'amplitude superposées autour d'un axe central, comme le Player
  d'Exec Modernisé ;
- un double mètre stéréo à LED rondes, gradué en décibels, et deux faders L/R
  sur toute la hauteur du panneau droit, placés contre leur mètre respectif.

Le chargement ajoute les fichiers et leurs titres à la liste. Un clic sur une
piste ou sur Lecture démarre sa lecture ; Pause et Arrêt contrôlent la piste
active. Le spectre et les mètres suivent les échantillons de la piste en cours
de lecture.

## Organisation

```text
apps/play-desktop/       fenêtre native, toolbar et composition egui
crates/play-core/        document, pistes, transport et état du spectre
crates/play-media/       inspection d'images et analyse spectrale stéréo
crates/play-ui/          scènes et contrats de panneaux portables
assets/fonts/             polices FreeSans et Material Icons
assets/texture/           textures des boutons de la barre d'outils
docs/                     architecture et décisions du projet
```

## Bibliothèques

| Besoin | Bibliothèque | État | Rôle |
|---|---|---|---|
| Fenêtre et événements | `winit` + `egui-winit` | actif | fenêtre native et événements |
| Interface | `egui` | actif | widgets, panneaux et peintres |
| Rendu GPU | `egui-wgpu` | actif | rendu de l'interface |
| Modèle | `serde` | actif | contrats sérialisables dans `play-core` et `play-ui` |
| Images | `image` | actif | textures PNG et inspection RGBA |
| Métadonnées | `lofty` | actif | lecture des jaquettes intégrées aux FLAC |
| Analyse | traitement temporel | actif | analyse stéréo en 64 bandes |
| Sélection de fichiers | `rfd` | actif | fenêtre native de chargement des musiques |
| Audio | `rodio` + `cpal` + `symphonia` | actif | décodage et lecture des fichiers |
| Rééchantillonnage | `rubato` | prévu | adaptation des flux audio |
| Persistance | `ron` | prévu | sauvegarde des sessions et réglages |

## Démarrage

Le projet utilise la toolchain stable déclarée dans `rust-toolchain.toml`.

Depuis la racine du workspace :

```bash
cargo run -p play-desktop
cargo check --workspace
cargo test --workspace
cargo fmt --all -- --check
```

Le binaire s'appelle `play`. Dans RustRover, ouvrir le `Cargo.toml` racine et
utiliser la configuration `Play Desktop` située dans
`.idea/runConfigurations/Play_desktop.xml`.

## Suite prévue

1. ajouter la persistance RON des sessions ;
2. valider le packaging et le rendu sur Linux, Windows et macOS.
