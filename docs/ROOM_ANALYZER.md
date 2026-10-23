# Room Analyzer

`RoomAnalyzer` est l'analyseur de fréquence mono de Play. Il écoute l'entrée
audio sélectionnée et utilise la plus récente fenêtre d'échantillons pour
mettre à jour le graphe en temps réel.

## Signal et résolution

Le flux est converti en mono avant l'analyse. Le graphe utilise 256 bandes,
avec une plage fréquentielle de 16 Hz à 23 kHz. Les niveaux sont affichés sur
une échelle positive de 0 à 110 dB afin de faciliter la lecture d'un signal
capté dans une pièce.

La fenêtre d'analyse est recalculée à chaque image à partir du buffer partagé
avec le thread audio. Les entrées stéréo sont donc réunies pour le Room
Analyzer, tandis que l'analyseur du mode Player conserve ses deux canaux.

## Entrée audio

Le panneau permet de choisir une entrée parmi les périphériques disponibles
via `rodio::microphone`, puis de rafraîchir la liste. Une erreur d'ouverture est
affichée directement dans le panneau. Une interface USB comme une Scarlett
Solo ou une pédale exposant une entrée audio système peut être sélectionnée si
elle est publiée par le système d'exploitation.

## Architecture

1. `rodio::microphone` fournit les échantillons ;
2. le thread audio les pousse dans `LiveAnalysisBuffer` ;
3. `play-desktop` extrait la fenêtre récente et la convertit en mono ;
4. `play-media::SpectrumAnalyzer::analyze_with_band_count` calcule les 256
   niveaux ;
5. le panneau dessine les bandes, les repères de fréquence et l'échelle en
   décibels.

Le Room Analyzer et le Live Tablature partagent la capture audio, mais chacun
calcule ses propres données d'affichage afin que l'analyse de fréquence reste
indépendante de la détection de hauteur.

