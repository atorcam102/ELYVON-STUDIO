# Elyvon Studio

**Elyvon Studio** es la estación de trabajo de audio digital (DAW) de Elyvon y la
primera aplicación de la futura suite creativa Elyvon. Es una aplicación de
escritorio nativa (Rust + Tauri 2) con interfaz React/TypeScript, que funciona
sin conexión y se distribuye con instaladores independientes por sistema y
arquitectura.

> **Estado: Fase 1 (v0.1.0).** Están implementados la arquitectura, la interfaz
> principal, la gestión de proyectos y la infraestructura de compilación y
> actualizaciones. **El motor de audio todavía no existe**: reproducción,
> grabación y medidores reales aparecen como no disponibles. Ver
> [ROADMAP](docs/ROADMAP.md).

## Qué hace hoy

| Área | Estado |
| --- | --- |
| Pistas de audio, instrumento y MIDI: crear, renombrar, duplicar, borrar, reordenar, color, mute/solo, selección múltiple | ✅ |
| Timeline: regla con compases, ciclo, cursor, zoom, cuadrícula con snap, mover regiones (tiempo y pista), crear regiones MIDI vacías | ✅ |
| Inspector, mezclador (volumen, pan, mute, solo), vista previa de piano roll (solo lectura), navegador de archivos | ✅ |
| Proyectos `.elyvon`: nuevo, abrir, guardar (atómico), guardar como, deshacer/rehacer, aviso de cambios sin guardar, recuperación tras fallo | ✅ |
| Menús nativos y atajos (⌘ en macOS, Ctrl en Windows/Linux), preferencias, logs | ✅ |
| Actualizaciones firmadas vía GitHub Releases, siempre con confirmación | ✅ (requiere configurar la clave de firma, ver [RELEASES](docs/RELEASES.md)) |
| Reproducción, grabación, edición de audio, plug-ins, instrumentos | ⏳ fases siguientes |

Los controles que dependen del motor de audio están deshabilitados y lo
indican; los medidores del mezclador solo muestran una simulación si se activa
explícitamente ("Simulate meters").

## Plataformas

| Sistema | Arquitectura | Instaladores |
| --- | --- | --- |
| Windows 10/11 | x64, x86, ARM64 | NSIS `.exe` (+ `.msi` en x64/x86) |
| macOS 11+ | Intel, Apple Silicon | `.dmg` (+ `.app.tar.gz` para actualizaciones) |
| Ubuntu 22.04+ | x64, ARM64 | `.deb`, `.AppImage` |

El estado real de compilación de cada objetivo lo muestra el workflow
[CI](.github/workflows/ci.yml) (job *Build*). No se considera compatible un
objetivo cuya compilación no esté en verde.

## Desarrollo rápido

Requisitos: Rust estable (1.88+), Node.js 22.12+, y en Linux las bibliotecas de
WebKitGTK (ver [CONTRIBUTING](CONTRIBUTING.md)).

```bash
npm ci
npm run tauri dev        # aplicación con recarga en caliente
npm test                 # pruebas del frontend (Vitest)
cargo test --workspace   # pruebas Rust (+ regenera src/bindings)
npm run tauri build      # instaladores para la plataforma actual
```

## Estructura

```
crates/elyvon-core      tiempo musical, colores, E/S atómica (reutilizable por la suite)
crates/elyvon-project   modelo, formato .elyvon, ediciones, undo, recuperación
crates/elyvon-audio     contratos del futuro motor de audio (sin motor)
crates/elyvon-update    política de actualizaciones, plataformas, manifiesto, CLI elyvon-release
src-tauri/              shell de escritorio: comandos IPC, menús, ajustes, updater
src/                    interfaz React (features/, components/, state/, styles/)
e2e/                    pruebas end-to-end contra el binario real (WebDriver)
docs/                   arquitectura, formato de proyecto, releases, roadmap
```

## Documentación

- [Arquitectura](docs/ARCHITECTURE.md)
- [Formato de proyecto `.elyvon`](docs/PROJECT_FORMAT.md)
- [Releases, firma y actualizaciones](docs/RELEASES.md)
- [Roadmap](docs/ROADMAP.md)
- [Contribuir](CONTRIBUTING.md)
- [Changelog](CHANGELOG.md)

## Privacidad

Sin telemetría. La única petición de red es la comprobación de actualizaciones,
que puede desactivarse en *Settings › Updates*.

## Licencia

© 2026 Elyvon. Todos los derechos reservados. No se ha elegido todavía una
licencia de distribución; las dependencias directas tienen licencias permisivas
(MIT/Apache-2.0) compatibles con distribución comercial.

## Referencia visual del frontend

La reproducción visual de las capturas está disponible en `/reference.html`
durante `npm run dev` y en la compilación de producción. Es una página de
referencia con datos de muestra estáticos (mezclador, biblioteca, Live Loops,
secuenciador y acordes), separada del punto de entrada de la aplicación Tauri.
El diseño se aplica directamente a los componentes de la aplicación nativa:
transporte con LCD, herramientas del arreglo, cabeceras compactas, mezclador
y biblioteca lateral izquierda. La página `/reference.html` se conserva solo
para comparación. La aplicación mantiene las acciones y los datos existentes
de Tauri. Los estilos principales están en `src/styles/reference-chrome.css`.
Las regiones de audio de la sesión demo muestran una ilustración estática
rotulada «Demo preview · No audio file»; no generan sonido ni sustituyen
señal real. Las regiones sin archivo de proyectos normales siguen vacías.

Las compilaciones de instaladores de CI se limitan por ahora a Windows x64 y
x86. `Publish Windows preview` publica los instaladores existentes de la
ejecución de CI indicada y documenta su commit de origen; no los presenta
como una compilación del frontend posterior.
