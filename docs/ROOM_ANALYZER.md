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

## Profil microphone et compensation

Le bouton de réglage de la barre d'outils basse ouvre le menu `Profil du
microphone`. Trois choix sont disponibles : `Aucune correction`, `Behringer
ECM8000 — courbe nominale` et `Shure Beta 58A — micro dynamique`. Le profil
ECM8000 est sélectionné par défaut pour le micro de mesure utilisé avec la
Scarlett Solo.

Quand le profil ECM8000 est actif, chaque bande est corrigée après l'analyse
FFT. La correction est interpolée en fréquence logarithmique à partir d'une
courbe nominale prudente : elle reste proche de 0 dB dans le médium et réduit
progressivement l'extrémité aiguë. Le menu affiche clairement qu'il s'agit
d'une courbe nominale : la réponse réelle varie selon l'exemplaire et une
calibration individuelle (`.cal`) est nécessaire pour une mesure de laboratoire
ou une mesure SPL fiable. `Aucune correction` permet de comparer directement le
signal brut de l'entrée.

Le profil Shure Beta 58A est surtout destiné à `LiveTablature` : le Beta 58A
est un micro dynamique et ne nécessite pas le 48 V. Dans `RoomAnalyzer`, il
laisse le signal sans compensation de courbe afin de ne pas appliquer la
correction de l'ECM8000 à un autre microphone.

## Entrée audio

Le panneau permet de choisir une entrée parmi les périphériques disponibles
via `rodio::microphone`, puis de rafraîchir la liste. Une erreur d'ouverture est
affichée directement dans le panneau. Une interface USB comme une Scarlett
Solo ou une pédale exposant une entrée audio système peut être sélectionnée si
elle est publiée par le système d'exploitation.

Quand une entrée contient `Scarlett` ou `Focusrite` dans son nom, elle est
préférée automatiquement lors de la première détection. La Scarlett Solo 3e
génération est ainsi le chemin prévu pour un ECM8000 branché en XLR. Le micro
de mesure doit recevoir l'alimentation fantôme 48 V de l'interface. Le niveau
d'entrée doit rester assez bas pour éviter la saturation ; RoomAnalyzer mesure
le signal reçu, mais ne remplace pas une calibration SPL avec une source et une
référence étalonnées.

## Bruit rose

En entrant dans `RoomAnalyzer`, Play ouvre la sortie audio par défaut et y
joue en boucle le fichier stéréo `assets/son/bruit_rose.wav` à niveau modéré
si aucune piste n'est en cours de lecture.
Quand l'entrée choisie est une Scarlett ou une Focusrite, Play cherche aussi
sa sortie correspondante ; un casque branché sur la sortie casque de la
Scarlett peut donc servir de source de test. Le graphe mesure toutefois le
signal reçu par l'ECM8000, pas le signal envoyé directement au casque. Le
bouton de volume de la barre d'outils basse arrête ou relance le signal de
test ; le bouton microphone arrête ou relance la capture. Les faders du panneau
stéréo à droite règlent le volume du bruit rose ou de la piste en cours. Si une
piste joue déjà, elle reste audible et devient la source observée par le
microphone ; le bandeau supérieur rappelle la piste et sa position.

**L'ECM8000 nécessite l'alimentation fantôme 48 V.** L'activer sur la Scarlett
Solo avant de démarrer la mesure et vérifier que `Niveau micro` réagit.

À l'ouverture de `RoomAnalyzer`, Play affiche une boîte de dialogue avant de
démarrer la capture et, si nécessaire, le bruit rose. Le bouton `J'ai activé le
48V — démarrer` confirme l'étape ; `Annuler` laisse la mesure arrêtée.

Pour une mesure de réponse de salle :

1. brancher l'ECM8000 sur une entrée micro de la Scarlett et activer le 48 V ;
2. sélectionner `Scarlett Solo USB` dans le menu `Entrée audio` et vérifier
   que `Niveau micro` quitte `-∞ dBFS` en parlant ou en tapotant le micro ;
3. pour une mesure de salle, utiliser une enceinte réelle ; avec un casque,
   placer l'ECM8000 près de l'écouteur qui joue le bruit rose ;
4. régler le gain sans écrêtage et observer les 256 bandes du graphe pour
   comparer les zones de la pièce ;
5. si `Niveau micro` reste à `-∞ dBFS`, vérifier le 48 V, le câble XLR et le
   périphérique d'entrée sélectionné.

Le bruit rose fourni est un signal de test à amplitude limitée. Les niveaux
affichés servent à comparer la réponse fréquentielle ; ils ne donnent pas une
mesure SPL absolue sans calibration du microphone, de l'interface et de la
chaîne de sortie.

## Architecture

1. `rodio::microphone` fournit les échantillons ;
2. le thread audio les pousse dans `LiveAnalysisBuffer` ;
3. `play-desktop` extrait la fenêtre récente et la convertit en mono ;
4. `play-media::SpectrumAnalyzer::analyze_with_band_count` calcule les 256
   niveaux ;
5. le profil microphone applique sa compensation nominale éventuelle ;
6. le panneau dessine les bandes, les repères de fréquence et l'échelle en
   décibels.

Le Room Analyzer et le Live Tablature partagent la capture audio. Le même
ECM8000 peut donc servir à capter une guitare acoustique ou le son d'un ampli
pour la tablature, à condition de tenir compte de son caractère
omnidirectionnel. RoomAnalyzer calcule ses 256 bandes indépendamment de la
détection de hauteur de LiveTablature ; le bruit rose est arrêté en quittant le
mode RoomAnalyzer.
