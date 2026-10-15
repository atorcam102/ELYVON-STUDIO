# Formato de proyecto `.elyvon` (esquema 1)

Un proyecto de Elyvon Studio es un único archivo JSON UTF-8 con extensión
`.elyvon`. Describe la canción (pistas, regiones, ajustes) y **referencia** los
archivos de audio en lugar de incrustarlos. No usa ni imita formatos de
terceros.

## Sobre

```json
{
  "format": "elyvon-studio-project",
  "schemaVersion": 1,
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
  "lengthBars": 64,                   // 1–9999; crece al mover/crear regiones al final
  "transport": {
    "tempo": 120,                     // BPM, 20–999
    "timeSignature": { "numerator": 4, "denominator": 4 },  // 1–32 / 1,2,4,8,16,32
    "key": { "tonic": "A", "mode": "minor" },               // C, C#, D … B · major|minor
    "loopRange": { "enabled": false, "start": 0, "end": 15360 }  // ticks, start < end
  },
  "tracks": [ /* Track, en orden de visualización (máx. 512) */ ]
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
  "content": { "type": "midi", "notes": [ { "pitch": 60, "velocity": 100, "start": 0, "length": 480 } ] }
  // o: { "type": "audio", "source": { "path": "Audio/take1.wav" } }   (source puede ser null)
}
```

- Las regiones `audio` solo pueden estar en pistas `audio`; las `midi`, en
  pistas `instrument` o `midi`.
- Notas: `pitch` 0–127, `velocity` 1–127, `start` relativo al inicio de la
  región, `length` > 0. Máximo 100 000 notas por región.
- `source.path` es relativo al archivo del proyecto cuando sea posible. En la
  Fase 1 no hay importación de audio: las regiones de audio de la sesión demo
  tienen `source: null` y la UI las identifica como marcadores sin archivo.

## Tiempo

Todas las posiciones y duraciones son enteros en **ticks**, con 960 ticks por
negra (PPQ). Un compás de 4/4 son 3840 ticks. Usar enteros evita errores de
redondeo acumulados al mover regiones.

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
