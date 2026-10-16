# Arquitectura de Elyvon Studio

## Principios

1. **El núcleo es nativo.** Todo el estado del proyecto, su validación, el
   historial de deshacer, la persistencia, el sistema de archivos y las
   actualizaciones viven en Rust. La interfaz web es solo presentación.
2. **La interfaz nunca muta el modelo.** Envía órdenes (`EditCommand`) y pinta
   la vista (`SessionView`) que devuelve el núcleo.
3. **El hilo de audio es sagrado.** Nada de asignaciones, liberaciones,
   bloqueos, E/S ni logging en el callback de audio (verificado por
   `crates/elyvon-audio/tests/rt_alloc.rs`).
4. **Honestidad funcional.** Lo que no existe se muestra deshabilitado y
   explicado; los datos de demostración solo los crea `elyvon_project::demo`.
5. **Pocas dependencias.** Sin servidores, sin servicios en la nube, sin
   telemetría.

## Capas

```
┌──────────────────────────── Webview (React + TS) ────────────────────────────┐
│ features/  topbar · timeline · tracks · inspector · mixer · editors · browser │
│ state/     zustand: project (espejo), ui, app, updates, dialogs              │
│ lib/ipc.ts invoke() tipado — tipos generados desde Rust (src/bindings)        │
└───────────────────────────────▲──────────────────────────────────────────────┘
                                │ Tauri IPC (comandos async + eventos + Channel)
┌───────────────────────────────┴───────── src-tauri ──────────────────────────┐
│ commands/  project · browser · system      menu.rs  settings.rs  updater.rs  │
│ state.rs   AppState { session, settings, updater, paths }                    │
└──────▲─────────────────────▲────────────────────────▲────────────────────────┘
       │                     │                        │
┌──────┴───────┐   ┌─────────┴──────────┐   ┌─────────┴──────────┐   ┌──────────────┐
│ elyvon-core  │◄──│  elyvon-project    │◄──────────────────────────│ elyvon-audio │
│ tiempo, mapa │   │ modelo, formato,   │   │  elyvon-update     │   │ motor RT,    │
│ de tempo, E/S│   │ ediciones, undo,   │   │ SemVer, plataformas│   │ medios, plan,│
│ atómica      │   │                    │   │ manifiesto, CLI    │   │ render, cpal │
└──────────────┘   │ recuperación, demo │   └────────────────────┘   └──────────────┘
                   └────────────────────┘
```

### Crates

| Crate | Responsabilidad | Reutilizable por la suite |
| --- | --- | --- |
| `elyvon-core` | Tiempo musical exacto en ticks (PPQ 960), tempo/compás/tonalidad validados, colores `#RRGGBB`, escritura atómica y lectura con límite de tamaño | Sí (color, E/S) |
| `elyvon-project` | `Project`/`Track`/`Region`, validación semántica, formato `.elyvon` con migraciones, `EditCommand` atómicos, historial por instantáneas, `ProjectSession` (dirty/undo/redo/guardado en dos fases), recuperación, sesión demo | No (específico de Studio) |
| `elyvon-audio` | Motor de tiempo real (`engine`), plan inmutable (`plan`), transporte y atómicos compartidos (`transport`), decodificación/remuestreo/picos (`media`), waveforms por región (`waveform`), render offline y export WAV (`render`), salida cpal y reloj interno (`device`), contratos MIDI/grafo | Parcial |
| `elyvon-update` | Decisión SemVer, matriz OS/arquitectura/instalador, nombres de artefactos, validación estricta de `latest.json`, verificación minisign, CLI `elyvon-release` | Sí (cada app se actualiza por separado) |
| `src-tauri` | Proceso de escritorio: comandos, menús, ventana, logs, ajustes, cierre seguro, actualizador | No |

Cambio respecto a la estructura sugerida: se añadió `elyvon-update` como crate
propio para poder probar toda la política de actualización sin Tauri ni red, y
para que el pipeline de releases use exactamente las mismas reglas (nombres,
claves de plataforma, validación) que la aplicación.

## Flujo de una edición

1. El usuario arrastra una región. La UI muestra una vista previa local
   (transform CSS) sin tocar el modelo.
2. Al soltar, envía `project_apply({ type: "moveRegions", ids, deltaTicks, trackOffset })`.
3. `ProjectSession::apply` clona el proyecto (instantánea para deshacer), aplica
   la orden y **revalida todo el proyecto**. Si algo falla, restaura la copia:
   una edición nunca queda a medias.
4. Si el contenido cambió, se registra en el historial y se genera un nuevo
   `state_id`; `dirty = state_id != saved_state_id`, así que deshacer hasta el
   estado guardado deja el proyecto limpio.
5. Devuelve `ApplyResult { view, outcome }`. La UI reemplaza su espejo.

Las llamadas de proyecto se serializan en una cola en el frontend
(`state/project.ts → enqueue`), de modo que las respuestas se aplican en el
orden en que se hicieron las ediciones. Los gestos continuos (faders, arrastres)
se confirman una sola vez al soltar: un paso de deshacer por gesto.

## Hilos y rendimiento

- **Hilo principal (UI/event loop):** solo la ventana y el webview. Todos los
  comandos Tauri son `async`, por lo que se ejecutan en el pool de Tokio y no en
  el hilo principal.
- **E/S de disco:** siempre en `spawn_blocking`. El guardado se hace en dos
  fases (`begin_save` bajo el mutex → escritura sin el mutex → `complete_save`),
  así una escritura lenta no bloquea otras órdenes. Las ediciones hechas durante
  el guardado mantienen el proyecto como modificado.
- **Hilo de audio** (callback de cpal o reloj interno): ejecuta
  `Engine::process`. Recibe órdenes de transporte y planes nuevos por colas SPSC
  preasignadas (`rtrb`) y devuelve los planes sustituidos por otra cola para
  liberarlos en el hilo de control. Publica posición, estado y picos con
  atómicos. Nunca toma el mutex de la sesión.
- **Hilo de control** (`src-tauri/src/audio.rs`): tras cada edición, deshacer o
  apertura reconstruye el `PlaybackPlan` (mapa de tempo → frames, segmentos de
  clip sin solapes con crossfades, notas MIDI, ciclo, metrónomo) y lo publica.
  Orden de bloqueo: sesión → motor → medios → carga.
- **Hilo de medios**: decodifica (Symphonia), remuestrea (rubato) y calcula la
  pirámide de picos; luego re-sincroniza el plan y emite `media://status`.
- **Publicador** (~30 Hz): emite `audio://position` solo cuando cambia y
  `audio://meters` mientras suena. La UI dibuja el playhead con esos eventos;
  no usa temporizadores propios para avanzar el tiempo.
- **Timeline:** un único contenedor de scroll nativo; regla y cabeceras
  `position: sticky`; cuadrícula dibujada con fondos CSS teselados (sin DOM por
  línea); solo se renderizan las regiones visibles; las notas MIDI de cada
  región se dibujan en un SVG memorizado.
- **Waveforms:** cada región pide al núcleo solo las columnas visibles
  (`media_region_peaks`, binario); React nunca recibe muestras. Caché LRU en la UI.
- **Reposo:** sin temporizadores periódicos en la UI. El publicador nativo no
  emite nada mientras el transporte está parado y sin cambios. La comprobación de
  actualizaciones es una sola petición al arrancar, opcional.

## Persistencia y recuperación

- Formato: ver [PROJECT_FORMAT.md](PROJECT_FORMAT.md).
- Guardado atómico: archivo temporal en el mismo directorio → `fsync` →
  `rename` → `fsync` del directorio (Unix). Un corte de luz deja el archivo
  anterior o el nuevo, nunca uno truncado.
- Recuperación: con cambios sin guardar, la UI pide al núcleo una instantánea
  tras 15 s sin actividad (`recovery_snapshot`). Se borra al guardar, descartar
  o salir limpiamente. Si existe al arrancar, se ofrece restaurarla.
- Preferencias (`settings.json` en el directorio de configuración de la
  plataforma): campos con valores por defecto; un archivo corrupto se aparta como
  `settings.json.corrupt` y la aplicación arranca igualmente.

## Cierre seguro

- Cerrar la ventana, *Quit* del menú o una salida pedida por el sistema pasan
  por `request_close`: si hay cambios sin guardar se cancela y se emite
  `app://close-requested`; la UI pregunta Guardar / No guardar / Cancelar.
- `app_quit`, `app_restart`, `project_new/open` y `update_install` se niegan en
  el núcleo (`ErrorCode::UnsavedChanges`) si hay cambios y no se indicó
  explícitamente descartarlos. La protección no depende de la interfaz.

## Actualizaciones

Resumen (detalle en [RELEASES.md](RELEASES.md)):

1. Comprobación opcional al arrancar (5 s después, una vez) o manual desde
   *Settings › Updates* / menú *Check for Updates…*.
2. El plugin oficial `tauri-plugin-updater` descarga `latest.json` por HTTPS.
3. `elyvon_update::decide` decide: no hay degradaciones; una instalación estable
   nunca pasa a una pre-release.
4. Se elige solo la entrada `{os}-{arch}-{instalador}` exacta y, además, se
   comprueba que el nombre del archivo corresponde a esa plataforma y versión.
5. Descarga cancelable con progreso; el plugin verifica la firma minisign con la
   clave pública embebida y `requireSignedVersion` vincula la firma a la versión.
6. Instalar y reiniciar requieren confirmación y que no haya cambios sin guardar.

## Seguridad

- CSP estricta, `freezePrototype`, sin plugin de sistema de archivos expuesto al
  webview: los accesos a disco pasan por comandos propios.
- Capacidades Tauri mínimas (`src-tauri/capabilities/default.json`).
- Las notas de versión se muestran como texto plano.
- Nunca se desactiva TLS ni la verificación de firmas.

## Errores y logs

- Todas las órdenes devuelven `IpcError { code, message }` con códigos estables
  (`unsavedChanges`, `needsSaveAs`, `invalidProject`, `newerVersion`,
  `network`, `noRelease`, `updateRejected`, `cancelled`…).
- `tauri-plugin-log`: stdout + archivo rotativo (5 MB × 3) en el directorio de
  logs de la plataforma. Los pánicos y los errores de la UI (boundary,
  `window.onerror`) también se registran.

## Tipos compartidos

Los tipos IPC se definen una sola vez en Rust y `ts-rs` genera
`src/bindings/*.ts` al ejecutar `cargo test`. CI falla si los bindings
versionados no coinciden.

## Preparación para la suite Elyvon

- Tokens de diseño en `src/styles/tokens.css` y componentes genéricos en
  `src/components/` sin dependencias de dominio: extraíbles a un paquete
  `@elyvon/ui` cuando exista una segunda aplicación.
- `elyvon-core` y `elyvon-update` no contienen nada específico de audio
  (salvo el módulo `music`) y el nombre de producto de los artefactos es un
  parámetro (`ArtifactNaming`).
- Cada aplicación tendrá su propio identificador, clave de firma, endpoint de
  actualizaciones y releases. No hay launcher ni cuentas.

## Auditoría y contratos de Fase 02

El estado verificable y las brechas se mantienen en [STATUS](STATUS.md),
[FEATURE_MATRIX](FEATURE_MATRIX.md) y [CONTRACTS](CONTRACTS.md). Los contratos nuevos de
grafo/MIDI/jobs no implementan un motor ni se guardan en el esquema 1. Las decisiones están
en [ADR](ADR/README.md). La distribución actual sigue limitada a Windows x86/x64; los otros
targets configurados no equivalen a soporte probado. La Fase 02 queda parcial mientras haya
criterios sin evidencia de hardware, firma de producción o atribuciones de distribución.

## Motor temporal y clips (Fases 04–05)

Ver [ADR 0005](ADR/0005-lock-free-plan-engine-and-in-memory-media.md) y la
evidencia en [audit/PHASE-04-05.md](audit/PHASE-04-05.md). Resumen:

- Tiempo musical en ticks; tiempo de audio en frames; `TempoMap` convierte
  ticks ↔ segundos ↔ frames con cambios de tempo y de compás, sin acumulación.
- La salida del motor es función pura de la posición: renders idénticos con
  cualquier tamaño de bloque, ciclo sample-accurate en límites de bloque.
- Solapes en una pista: gana la región que empieza más tarde; con fade-in hay
  crossfade de igual potencia con la región cubierta.
- Los archivos fuente son inmutables; el audio decodificado vive en memoria
  (presupuesto 4 GiB / 512 MiB en 32 bits). El streaming desde disco no existe aún.
