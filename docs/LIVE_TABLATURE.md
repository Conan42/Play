# Live Tablature

`LiveTablature` transforme une entrée audio de guitare ou de voix en affichage
musical progressif. Il peut aussi analyser la sortie de la piste jouée par Play :
cette source suit le signal envoyé à la sortie audio active, donc à la Scarlett si
elle est sélectionnée comme sortie de lecture. Le mode microphone utilise le même
flux que `RoomAnalyzer` et renvoie l'entrée vers la sortie casque active pour
le monitoring. Les deux sources conservent une fenêtre temporelle de notes afin
de faire défiler la partition.

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

La hauteur du panneau détermine le nombre de systèmes visibles. Dans chaque
système, les notes avancent de gauche à droite selon leur instant de capture.
Lorsque la portée courante arrive au bord droit, la séquence continue sur la
portée suivante ; quand la hauteur disponible est pleine, la portée courante
reste visible en bas et les portées précédentes remontent automatiquement.
Le défilement est commun aux trois modes.

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
- la commande `Play/Pause` de la barre d'outils principale contrôle la piste
  audio et reste disponible dans ce mode ;
- `Play/Pause` de la barre basse suspend ou reprend le défilement de la
  tablature et la capture ;
- `Enregistrer les notes` contrôle l'ajout des notes dans l'historique ;
- `Exemple immédiat`, `Nouvel exercice` et `Effacer` contrôlent l'historique ;
- le sélecteur `Source analysée` choisit le microphone/entrée Scarlett ou la
  sortie de la piste Play ; le mode microphone active aussi le monitoring vers
  la sortie casque de la Scarlett ; le bouton `MON` permet de couper ou de
  réactiver ce retour sans interrompre la détection ; le bouton `MONO` duplique
  une source guitare mono dans les deux côtés du casque ;
- le choix de l'entrée audio permet d'utiliser un microphone, une interface
  USB ou une autre entrée exposée par le système ;
- le menu de profil microphone permet de sélectionner `Shure Beta 58A — micro
  dynamique` lorsqu'il est branché sur l'entrée XLR de la Scarlett Solo ; ce
  microphone dynamique ne nécessite pas l'alimentation fantôme 48 V ;
- le BPM et le métronome servent de référence visuelle pour les mesures.

La piste audio en cours reste audible lors de l'ouverture de `LiveTablature`.
Un bandeau au-dessus du panneau affiche son titre, son artiste, son état et sa
position de lecture. Le panneau mètre stéréo à droite reprend les niveaux du
micro et ses faders L/R règlent directement le volume du monitoring casque.

La détection reste monophonique pour la position de tablature : elle convient
donc au chant d'une mélodie comme `Shape of My Heart` et convertit chaque note
vocale en position équivalente sur le manche de guitare. La détection d'accord
fournit une indication complémentaire et ne remplace pas encore une
transcription polyphonique complète.
