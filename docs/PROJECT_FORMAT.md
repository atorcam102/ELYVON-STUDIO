# Formato de proyecto `.elyvon` (esquema 6)

Un proyecto de Elyvon Studio es un único archivo JSON UTF-8 con extensión
`.elyvon`. Describe la canción (pistas, regiones, ajustes) y **referencia** los
archivos de audio en lugar de incrustarlos. No usa ni imita formatos de
terceros.

## Sobre

```json
{
  "format": "elyvon-studio-project",
  "schemaVersion": 3,
  "savedWith": "0.1.0",
  "project": { … }
}
```

| Campo | Significado |
| --- | --- |
| `format` | Identifica el archivo independientemente de su extensión. Otro valor → "no es un proyecto de Elyvon Studio". |
| `schemaVersion` | Entero. Se incrementa en cada cambio incompatible de `project`. |
| `savedWith` | Versión de la aplicación que lo guardó (solo diagnóstico). |

Reglas de compatibilidad (`elyvon_project::format`):

- `schemaVersion` mayor que el soportado → se rechaza con un mensaje que pide
  actualizar la aplicación (nunca se interpreta a medias).
- Menor → se migra en memoria, paso a paso (`migrate(v1→v2→…)`), operando sobre
  el JSON para no necesitar tipos antiguos. Cada cambio de esquema añade un
  fixture en `crates/elyvon-project/tests/fixtures/`.
- Ausente o no numérico → archivo inválido.
- Tamaño máximo 64 MiB.

## `project`

```jsonc
{
  "id": "uuid",                       // identidad estable del proyecto
  "name": "My Song",                  // 1–128 caracteres, sin caracteres de control
  "sampleRate": 48000,                // 44100, 48000, 88200, 96000, 176400, 192000
  "lengthBars": 64,                   // 1–9999; crece al mover/crear regiones al final
  "transport": {
    "tempo": 120,                     // BPM, 20–999
    "timeSignature": { "numerator": 4, "denominator": 4 },  // 1–32 / 1,2,4,8,16,32
    "key": { "tonic": "A", "mode": "minor" },               // C, C#, D … B · major|minor
    "loopRange": { "enabled": false, "start": 0, "end": 15360 },  // ticks, start < end
    "tempoChanges": [ { "tick": 7680, "bpm": 140 } ],        // escalones, tick > 0 y creciente
    "meterChanges": [ { "bar": 4, "timeSignature": { "numerator": 7, "denominator": 8 } } ], // compás 5 (índice 0)
    "metronome": { "enabled": false, "volumeDb": -6, "accent": true },
    "countInBars": 0,                 // 0–4
    "preRollBars": 0,                 // 0–4
    "punch": { "enabled": false, "start": 0, "end": 15360 } // limita la ventana de grabación
  },
  "tracks": [ /* Track, en orden de visualización (máx. 512) */ ],
  "media": [ /* MediaAsset (máx. 4096) */ ],
  "markers": [ { "id": "uuid", "name": "Chorus", "tick": 30720 } ]  // máx. 1000
}
```

### Track

```jsonc
{
  "id": "uuid",
  "name": "Lead Vocal",
  "kind": "audio",            // audio | instrument | midi
  "color": "#19A7C0",
  "muted": false,
  "soloed": false,
  "volumeDb": -3.0,           // -96 (= −∞) … +6
  "pan": 0.0,                 // -1 … +1
  "regions": [ /* Region, ordenadas por inicio (máx. 4096) */ ]
}
```

Audibilidad: una pista en mute nunca suena; si alguna pista está en solo, solo
suenan las pistas en solo (y no en mute). La lógica está en
`Track::is_audible` y la usará el motor de audio.

### Region

```jsonc
{
  "id": "uuid",
  "name": "Chorus",
  "start": 30720,            // ticks desde el inicio de la canción
  "length": 15360,           // ticks, > 0
  "muted": false,
  "content": { "type": "midi", "notes": [ { "pitch": 60, "velocity": 100, "start": 0, "length": 480 } ] }
  // o: { "type": "audio", "media": "uuid" | null, "offset": 0, "gainDb": 0, "fadeIn": 0, "fadeOut": 0, "looped": false }
}
```

- Las regiones `audio` solo pueden estar en pistas `audio`; las `midi`, en
  pistas `instrument` o `midi`.
- Notas: `pitch` 0–127, `velocity` 1–127, `start` relativo al inicio de la
  región, `length` > 0. Máximo 100 000 notas por región.
- Audio: `media` referencia una entrada del pool por id (debe existir); `offset`,
  `fadeIn` y `fadeOut` están en frames **del archivo fuente** (a su propia tasa),
  así no dependen del tempo. `gainDb` −96…+24. `looped` repite el archivo de
  fin a inicio hasta el final de la región. `media: null` = marcador silencioso
  (sesión demo).
- Edición no destructiva: recortar el inicio avanza `offset`; dividir calcula el
  `offset` de la segunda mitad con el mapa de tempo. El archivo nunca se escribe.

### MediaAsset

```jsonc
{
  "id": "uuid",
  "name": "take1.wav",
  "path": "/ruta/absoluta/take1.wav",      // o relativa al directorio del proyecto
  "info": {                                // null si nunca se decodificó (migrado de v1)
    "sampleRate": 44100, "channels": 2, "frames": 88200,
    "format": "wave", "sizeBytes": 352844, "fingerprint": "8f3c2a1b4d5e6f70"  // FNV-1a 64
  }
}
```

Al abrir, cada archivo se decodifica en segundo plano. Si falta, sus regiones
quedan en silencio y la UI lo indica (con «Locate file…»); si la huella difiere,
se avisa de que el archivo cambió.

## Tiempo

Todas las posiciones y duraciones son enteros en **ticks**, con 960 ticks por
negra (PPQ). Un compás de 4/4 son 3840 ticks. Usar enteros evita errores de
redondeo acumulados al mover regiones. El motor convierte ticks a frames con el
mapa de tempo (`elyvon_core::tempo`) desde el inicio de cada segmento:
`frame = floor(segundos × tasa + 0,5)`.

## Migración 1 → 2

Automática al abrir (`format::v1_to_v2`): añade `sampleRate` 48000, mapas vacíos,
metrónomo apagado, count-in/pre-roll 0, punch desactivado, `markers` y `media`;
cada `source.path` distinto pasa al pool (`info: null`) y la región lo referencia
con offset 0, ganancia 0 dB, sin fades ni loop. Ninguna ruta se pierde
(fixture `tests/fixtures/v1-basic.elyvon`). Guardar escribe esquema 2; versiones
antiguas de la aplicación lo rechazan con "versión más nueva".

## Validación

Al abrir, cada archivo pasa por `validate()` antes de llegar a la sesión. Los
errores indican la ruta exacta del valor, por ejemplo
`tracks[2].regions[0].length: must be greater than zero`. Cada edición se
revalida igual, así que un proyecto guardado siempre cumple estas reglas.

## Escritura

Escritura atómica (temporal + `fsync` + `rename`). El JSON se guarda con
sangría para facilitar diffs y depuración.

## Recuperación

Las instantáneas de recuperación (`autosave.elyvon-recovery` en el directorio
de datos de la aplicación) envuelven un sobre de proyecto completo:

```json
{ "format": "elyvon-studio-recovery", "originalPath": "/ruta/song.elyvon", "savedAtUnix": 1792070000, "projectFile": { … } }
```


## Schema 3 — recorded takes and MIDI IO

Schema 2 migrates in memory with neutral defaults; schema 1 migrates through schema 2. Original
files/fixtures are not rewritten on open. Current saves use schema 3, so older applications refuse
them rather than silently discarding recording/routing fields.

Each track adds `io`: `armed`, zero-based `inputChannel`, `inputChannels` (1/2), `monitor`,
`midiInput`/`midiOutput` stable port choices, optional `midiChannel` input filter, optional
`outputChannel` override, and `midiMonitor`. Missing ports remain selected and unavailable;
they are never silently replaced by another physical port.

Regions add optional `capture` (`folder`, one-based `take`, `recordedAt` UTC Unix seconds) and
`midiEvents` (`tick` relative to region start, validated MIDI 1.0 `bytes`, maximum 256 bytes per
SysEx and 100000 events per region). Raw events are the playback/export source when present;
`content.notes` is the piano-roll projection. Splitting/trimming preserves source events and
chases/closes channel/key state at new region edges. Take selection changes region mute state
as a single undoable edit; source WAV/BWF files remain immutable.

Audio capture journals and `.partial.wav` files are outside the project JSON under the app's
recordings/project-id folder. Recovery repairs channel-aligned frames, writes a valid BWF header,
and finalizes without overwriting another take. The project references committed media by ID/path.

> Schema 3 note (superseded by schema 4 below): raw note events are no longer a playback source.

## Schema 4 — editable notes and the mixer (Phases 08–09)

Schema 3 files migrate in memory; the source file is not rewritten
(`tests/fixtures/v3-midi.elyvon`). Current saves use schema 4.

**Notes.** `content.notes[]` is the authoritative note data: `id` (unique, non-zero within the
region), `pitch`, `velocity` 1–127, `channel` 0–15, `muted`, `start`, `length` (> 0) and
`quantizeOffset` (`start` minus the position before quantisation; 0 when unquantised, never
before the region). `midiEvents` holds only controller, pitch-bend, pressure, program and SysEx
events; note on/off messages there are rejected. Migration from schema ≤ 3 (and every MIDI
import/recording) pairs note on/off per channel and key first-in-first-out, ends held notes at the
region end, drops trailing all-notes-off/all-sound-off closers and keeps every other event.
Note-off velocity is not stored.

**Tracks** add `strip` and `folder`:

| Field | Meaning |
| --- | --- |
| `strip.output` | `{type:"master"}` or `{type:"aux", id}` |
| `strip.sends[]` | `target` aux, `levelDb` −96…+6, `preFader`, `muted`; one send per target, at most 8 |
| `strip.inserts[]` | `id`, `bypass`, `processor`: `trim` (−48…+24 dB), `utility` (invert L/R, swap, mono), `filter` (`highPass`/`lowPass`, 20 Hz…20 kHz, Q 0.3…10), `limiter` (ceiling −24…0 dB, look-ahead 0…20 ms, release 1…1000 ms); at most 8 |
| `strip.soloSafe` | stays audible while others are soloed |
| `strip.group` | optional mixer group id |
| `strip.format` | `auto` (mono for audio tracks playing only mono files), `mono`, `stereo` |
| `folder` | optional mixer folder id |

**Project** adds `mixer`: `auxes[]` (`id`, `name`, `color`, `volumeDb`, `pan`, `muted`, `soloed`,
`strip`), `groups[]` (`id`, `name`, `color`, `linkVolume`, `linkPan`, `linkMute`, `linkSolo`),
`folders[]` (`id`, `name`, `color`, `collapsed`), `master` (`volumeDb`, `muted`, `inserts`) and
`panLaw` (`constantPower` −3 dB, `constantPowerCompensated` 0 dB, `linear` −6 dB). At most 64
auxes, 32 groups and 64 folders. Every reference must exist and the output/send graph must be
acyclic; a file or edit with a cycle is rejected (no feedback mode exists).

## Schema 5 — automation, comps, freeze and editing (Phases 10–11)

Schema 4 files migrate in memory without being rewritten (`tests/fixtures/v4-mixer.elyvon`).
Every new field has a neutral default. Older applications refuse schema 5.

**Automation.** Tracks, auxes (`mixer.auxes[].automation`) and the master
(`mixer.master.automation`) have `automation: { mode, lanes[] }`; `mode` is `off`, `read`
(default), `touch`, `latch` or `write`. A lane is `{ param, points[] }`:

| `param` | Unit and range | Interpolation scale |
| --- | --- | --- |
| `{type:"volume"}`, `{type:"sendLevel", aux}` | dB −96 (silence) … +6 | linear in dB |
| `{type:"pan"}` | −1 … +1 (not on the master) | linear |
| `{type:"mute"}`, insert `bypass` | 0 or 1 (always stepped) | — |
| `{type:"insert", id, param:"trimGain"}` | −48 … +24 dB | linear in dB |
| `… "filterCutoff"` / `"filterQ"` | 20 Hz … 20 kHz / 0.3 … 10 | logarithmic |
| `… "limiterCeiling"` / `"limiterRelease"` | −24 … 0 dB / 1 … 1000 ms | linear / logarithmic |

Points are `{ id, tick, value, curve }`, sorted by `tick` (several points may share a tick: a
jump), at most 100 000 per lane and 64 lanes per channel. `curve` shapes the segment to the next
point: `{type:"linear"}`, `{type:"step"}` or `{type:"bezier", c1, c2}` (control values 0…1;
1/3, 2/3 is linear). The fraction between two points is measured in ticks. Regions may have
`automation: [lane]` with ticks relative to the region start (inside the region, at most 16
lanes); it replaces the track lane inside the region. A lane must address an existing send or
insert parameter of its channel; external MIDI tracks have no automation.

**Audio regions** add `reversed` (plays `[offset, offset + heard length)` backwards; not with
`looped`) and `fadeCurve`: `equalPower` (default), `linear` or `sCurve`; it also shapes the
crossfade a region makes with the region it covers.

**Tracks** add `comps[]` — `{ folder, segments: [{ start, end, take }], crossfadeMs (0…200),
flattened, regions[] }` (a take folder's composite; segments ordered and non-overlapping) — and
`freeze: { media, start, fingerprint } | null` (a pre-fader render in the media pool; the
fingerprint identifies the material it was rendered from).

**Mixer groups** add `linkEdit` (phase-locked region editing across members).

## Schema 6 — native effects (Phases 12–13)

Schema 5 files migrate by bumping the version (they contain no effects). Older applications
refuse schema 6 instead of dropping effects.

**Effect inserts.** An insert's `processor` may be `{ "type": "effect", "kind", "params",
"sidechain"?, "alternate"? }`. `kind` is one of `parametricEq`, `compressor`, `gate`,
`stereoTool`, `testGenerator`, `delay`, `reverb`, `convolutionReverb`, `chorus`,
`flanger`, `phaser`, `tremolo`, `rotary`, `distortion`, `bitcrusher`, `pitchShifter`.
`params` maps parameter keys to numbers in their own unit (dB, Hz, ms, %, or the index of a
choice; toggles are 0/1). On load unknown keys are ignored, missing keys take the default and
values are clamped into range. `sidechain` (compressor, gate) is a track id; a track cannot key
itself and track-to-track keys must not form a cycle (at most 16 distinct key tracks).
`alternate` is the stored other half of an A/B comparison (same key format). The parameter
tables (keys, ranges, defaults, units, choices) are `elyvon-project/src/effects.rs` and are
returned at run time by `effects_catalog`.

**Effect automation.** `{type:"insert", id, param: { "effect": { "kind", "index" } }}`
addresses parameter `index` of the effect's table (the index never changes meaning). Choices
and toggles are stepped; logarithmic parameters interpolate on a log scale. Parameters that
rebuild the processor (convolution space and IR length, distortion oversampling) are not
automatable.

**Filters** add `"mode": "bandPass"` and `"notch"`.

**Presets** (user files, `<app data>/presets/<kind>/<name>.elyvonpreset`):
`{ "format": "elyvon-effect-preset", "version": 1, "kind", "name", "params": { key: number } }`.
Readers accept every version up to theirs and refuse newer ones with a clear error.
