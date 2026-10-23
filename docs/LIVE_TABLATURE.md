# Live Tablature

`LiveTablature` transforme une entrée audio de guitare en affichage musical
progressif. Le mode utilise le même flux microphone que `RoomAnalyzer`, mais
conserve une fenêtre temporelle de notes afin de faire défiler la partition.

## Détection

À chaque fenêtre audio, `play-media` estime :

- la fréquence fondamentale ;
- la note MIDI et son écart en cents ;
- la position la plus grave possible sur les six cordes et les vingt premières
  cases ;
- la fondamentale et la qualité d'un accord lorsque plusieurs notes sont
  présentes.

Chaque note retenue conserve son instant, sa corde, sa case et sa hauteur MIDI.
Les répétitions sont limitées pour éviter de remplir l'affichage avec la même
note lorsque la corde est tenue.

## Présentations

Le sélecteur `Affichage` propose trois modes :

- **Tablature** : six lignes `e B G D A E` avec les numéros de cases ;
- **Portée** : portée classique à cinq lignes, clé de sol, notes détectées,
  altérations et lignes supplémentaires ;
- **Portée + tablature** : les deux systèmes utilisent la même position
  horizontale pour chaque note.

La hauteur du panneau détermine le nombre de systèmes visibles. Le défilement
est commun aux trois modes et suit l'âge des notes depuis leur capture.

## Mesures et temps

La mesure affichée est actuellement `4/4`. Le BPM du panneau détermine la
durée d'un temps (`60 / BPM`) et d'une mesure. Les lignes fortes indiquent le
début des mesures (`M1`, `M2`, etc.) et les lignes plus discrètes indiquent les
temps intermédiaires. Les lignes sont dessinées à la même position sur la
portée et la tablature.

L'horloge démarre avec la première note capturée. Quand l'utilisateur met
`Play` en pause, l'origine temporelle et les notes sont décalées ensemble afin
que les mesures reprennent au même endroit. `Effacer` recommence une nouvelle
séquence de mesures.

## Commandes

- `TABLATURE` et `STRUMMING` changent la vue pédagogique ;
- `Play/Pause` suspend ou reprend le défilement et la capture ;
- `Enregistrer les notes` contrôle l'ajout des notes dans l'historique ;
- `Exemple immédiat`, `Nouvel exercice` et `Effacer` contrôlent l'historique ;
- le choix de l'entrée audio permet d'utiliser un microphone, une interface
  USB ou une autre entrée exposée par le système ;
- le BPM et le métronome servent de référence visuelle pour les mesures.

La détection reste monophonique pour la position de tablature. La détection
d'accord fournit une indication complémentaire et ne remplace pas encore une
transcription polyphonique complète.

