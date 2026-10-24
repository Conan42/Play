# BackingTrack

`BackingTrack` sert à jouer avec un groupe lorsque certains musiciens sont
absents. Chaque musicien possède une piste audio, appelée stem, qui est lue en
parallèle avec les autres stems sur la sortie active de Play.

## Projet vide au démarrage

Le projet démarre avec une liste vide. Le bouton dossier permet de charger une
ou plusieurs pistes audio, et le bouton `+` permet de créer une piste vide.
Chaque ligne possède un rôle éditable.

Le bouton dossier de la barre d'outils permet de sélectionner une ou plusieurs
pistes audio. Le nom de piste est déduit automatiquement du fichier : pour un
fichier nommé `Space Oddity - Batterie.wav`, Play affiche `Batterie` comme nom
de piste et place `Space Oddity - Batterie.wav` dans la cellule texte. Cette
cellule reste éditable.

Chaque ligne affiche aussi une forme d'onde compacte de son échantillon. Le
panneau de droite conserve les vumètres stéréo et les deux faders master, avec
une échelle de -60 dB à +10 dB. L'analyseur de spectre est fixé dans la partie
basse de la fenêtre afin que la liste des stems garde l'espace restant et
puisse défiler comme une liste de DAW.

## Mixage dans la piste

Chaque ligne fonctionne comme une tranche de mixage simplifiée :

- `M` coupe le stem ;
- `S` isole le stem et coupe les autres stems non solo ;
- `Vol` règle le niveau de la piste de -60 dB à +10 dB ;
- `Pan` règle la position gauche, centre ou droite.

Ces commandes restent dans la ligne du stem afin que le panneau droit puisse
continuer à afficher les mètres des autres modes.

La liste s'adapte aux petites fenêtres avec un défilement horizontal et
vertical. Les commandes de mixage restent ainsi entièrement accessibles,
notamment les boutons `M`, `S` et suppression.

## Lecture

La piste `Réf` permet de désigner la capture complète du morceau comme
référence temporelle. Après avoir chargé la référence et les stems individuels,
`Synchroniser avec Référence` compare leurs attaques audio, calcule le
décalage de chaque fichier et le conserve dans la session. Le décalage est
appliqué à la lecture sans modifier les fichiers WAV. La colonne `Sync` affiche
le décalage calculé en secondes. L'analyse s'exécute en arrière-plan ; une
petite fenêtre affiche une jauge avec le nombre de pistes traitées et empêche
la fenêtre de paraître bloquée pendant le calcul.

La barre d'outils supérieure et la ligne partagent le transport du BackingTrack.
Les stems chargés sont préparés avant le démarrage, puis leurs lecteurs sont
lancés ensemble. Pause, reprise et arrêt s'appliquent à toutes les pistes. Le
tempo du projet est conservé pour les prochaines fonctions de décompte,
boucle et métronome ; la version actuelle ne déforme pas encore les fichiers
pour les synchroniser à un autre BPM.

Les noms, rôles, chemins, référence, décalages, volumes, panoramiques, mute,
solo, tempo, position de lecture et faders master sont sauvegardés dans un
projet `.ron`. Le menu principal permet de créer un projet vide ou d'en ouvrir
un autre. Si le projet courant contient des modifications non enregistrées,
Play propose de les enregistrer avant de changer de projet, de mode ou de
fermer la fenêtre.
