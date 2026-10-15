# Roadmap

Las fases son incrementales: cada una deja una aplicación publicable. Las fechas
se fijan al cerrar la fase anterior.

## Fase 1 — Fundamentos (v0.1.x) ✅

Arquitectura Rust + Tauri, interfaz principal, formato de proyecto versionado,
undo/redo, recuperación, menús nativos, CI de 7 plataformas, releases en
borrador, actualizaciones firmadas.

Pendiente para cerrar la fase: configurar la clave de firma y publicar v0.1.0
(ver [RELEASES.md](RELEASES.md)); certificados de macOS/Windows.

## Fase 2 — Motor de audio (v0.2)

- `elyvon-audio-engine`: backend de dispositivos (CoreAudio, WASAPI, ALSA/
  PipeWire/JACK) mediante una capa de abstracción propia; selección de
  dispositivo, frecuencia de muestreo y tamaño de buffer.
- Hilo de tiempo real con prioridad, grafo de procesamiento preasignado,
  intercambio atómico del grafo, colas lock-free UI↔audio, medidores reales.
- Transporte: reproducir/parar/ciclo/localizar con reloj de muestra; metrónomo.
- Importación de audio (WAV/AIFF/FLAC/MP3) con decodificación en hilos de
  trabajo, caché de formas de onda (peaks) en disco y en la región.
- Mezclador real: volumen, pan (ley de potencia constante ya definida), mute/
  solo, bus master, medidores pico/RMS.
- Grabación de audio con compensación de latencia; armado de pistas (R).

## Fase 3 — MIDI e instrumentos (v0.3)

- Entrada/salida MIDI por dispositivo, grabación MIDI, pistas MIDI externas.
- Editor piano roll completo: dibujar, mover, redimensionar, cuantizar,
  velocidad, automatización básica.
- Primer instrumento integrado (sintetizador y sampler) en Rust.
- Navegador de loops con previsualización sincronizada a tempo/tonalidad.

## Fase 4 — Plug-ins y edición avanzada (v0.4)

- Hosting CLAP y VST3 (y AU en macOS) en proceso aislado, escaneo en segundo
  plano, compensación de retardo.
- Efectos integrados (EQ, compresor, reverb, delay) e inserciones/envíos/buses.
- Editor de audio: recorte, fundidos, estiramiento de tiempo, comping de tomas.
- Automatización por pista y por región.

## Fase 5 — Producción (v0.5+)

- Exportación/bounce (WAV, FLAC, MP3) y renderizado offline más rápido que
  tiempo real.
- Marcadores, pistas de arreglo, carpetas/stacks de pistas.
- Atajos configurables, tema claro, localización (es, en…).
- Optimización: diffs incrementales en el IPC en lugar de la vista completa,
  virtualización vertical de pistas para proyectos grandes.

## Suite Elyvon

Cuando exista una segunda aplicación (Photo, Motion, Cut, Write, Draw) se
extraerán `src/components` + `src/styles/tokens.css` a un paquete de UI
compartido y `elyvon-core`/`elyvon-update` a un repositorio común. Cada
aplicación mantendrá identificador, clave de firma, canal de actualizaciones y
releases propios. No se planea launcher ni cuenta obligatoria.
