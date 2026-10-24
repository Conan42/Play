# Ripper

`Ripper` capture une sortie audio exposée par le système et l'enregistre en
WAV PCM 24 bits. Il sert notamment à récupérer le mix entendu dans le casque
pendant la lecture d'un backing track Songsterr, afin de l'importer ensuite
dans `BackingTrack`. L'analyseur stéréo L/R et les mètres suivent la même
source `Monitor` pendant la capture.
Les faders L/R du panneau droit règlent l'atténuation appliquée au WAV et les
mètres affichent le niveau réellement capturé.

## Utilisation

Sous Linux avec PipeWire, Ripper capture directement la sortie audio avec
`pw-record`. Lorsqu'une interface Scarlett ou Focusrite est disponible, elle
est sélectionnée automatiquement pour éviter le périphérique générique
« Default Audio Device » qui peut être silencieux :

1. vérifier que la Scarlett est branchée et reçoit bien le son ;
2. ouvrir le mode `Ripper` dans le menu principal ;
3. vérifier dans le menu de source que la sortie Scarlett est affichée ;
4. démarrer la capture, lancer Songsterr, puis arrêter la capture ;
5. ouvrir `BackingTrack`, cliquer sur l'unique bouton dossier et sélectionner le
   WAV créé pour l'ajouter comme stem.

Le Ripper utilise le sink PipeWire Scarlett/Focusrite et force la capture de
ses ports monitor internes (`stream.capture.sink=true`) ; aucune source
`Monitor` visible dans la liste des entrées n'est nécessaire. Si aucune
Scarlett ou Focusrite n'est détectée, Play utilise la sortie audio par défaut.
La commande `pw-record` doit être installée. Sur les autres plateformes, Play
utilise une entrée monitor exposée par le système lorsqu'elle est disponible.

Le Ripper enregistre le mix stéréo réellement envoyé à la sortie. Il ne peut
pas reconstruire les stems séparés des instruments.
