# IR di casse

Metti qui i file WAV con le risposte all'impulso (IR) di casse vere. Poi in un preset:

```toml
[[block]]
type = "cab"
ir = "irs/nome-del-file.wav"   # percorso relativo alla cartella openPedal
max_ms = 80                     # opzionale: taglia le IR troppo lunghe
level_db = 0                    # opzionale
```

Formati letti: WAV mono o stereo (si usa il canale sinistro), float a 32 bit o PCM a 16/24/32 bit,
qualsiasi frequenza di campionamento (viene convertita). Il volume viene normalizzato da solo.

Ci sono molti pacchetti di IR gratuiti (cerca "free guitar cab IR"). Controlla la licenza di ognuno.
