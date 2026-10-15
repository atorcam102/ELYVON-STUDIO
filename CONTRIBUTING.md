# Contribuir a Elyvon Studio

## Entorno

| Herramienta | Versión |
| --- | --- |
| Rust | estable ≥ 1.90 (`rustup`) |
| Node.js | ≥ 22.12 |
| Linux | `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev libxdo-dev libssl-dev libasound2-dev patchelf` |
| macOS | Xcode Command Line Tools |
| Windows | Visual Studio Build Tools (C++), WebView2 (incluido en Windows 10/11) |

```bash
npm ci
npm run tauri dev
```

## Comprobaciones (las mismas que CI)

```bash
npm run check              # versions, typecheck, lint, tests, build
npm run check:rust         # fmt, clippy, tests y drift de bindings
node scripts/dependency-inventory.mjs dependency-inventory.json
npm audit --audit-level=low
cargo deny --locked check  # 0.20.2; excepción documentada B-01
actionlint -shellcheck= -pyflakes= # 1.7.12
```

Pruebas end-to-end contra el binario real (Linux):

```bash
sudo apt install webkit2gtk-driver xvfb dbus-x11
cargo install tauri-driver --locked
npx tauri build --debug --no-bundle
E2E_SCREENSHOTS=e2e-screenshots xvfb-run -a dbus-run-session -- node --test e2e/app.e2e.mjs
xvfb-run -a dbus-run-session -- node e2e/measure.mjs
```

## Reglas de arquitectura

- **La lógica va en Rust.** La UI no calcula ni valida el modelo: envía
  `EditCommand` y pinta la `SessionView`. Nada de DSP en JavaScript.
- **Nueva edición del proyecto** = variante de `EditCommand` en
  `crates/elyvon-project/src/edit.rs` + etiqueta para deshacer + pruebas. La
  validación global se ejecuta tras cada edición; si hace falta una regla nueva,
  añádela a `validate.rs`.
- **Cambiar el formato de proyecto**: incrementa `CURRENT_SCHEMA_VERSION`, añade
  la migración en `format::migrate`, un fixture del esquema anterior en
  `tests/fixtures/` y actualiza `docs/PROJECT_FORMAT.md`. Los fixtures existentes
  nunca se modifican.
- **Comandos IPC** siempre `async`; E/S en `spawn_blocking`; nunca mantener el
  mutex de la sesión durante E/S.
- **Futuro código de audio en tiempo real**: sin asignaciones, bloqueos, E/S ni
  logging en `AudioProcessor::process`. Parámetros vía atómicos o colas
  preasignadas.
- **Honestidad**: una función que no existe se muestra deshabilitada con su
  motivo. Los datos de demostración solo en `elyvon_project::demo`.
- **Diseño**: usa los tokens de `src/styles/tokens.css`; el adaptador de
  `src/components/Icon.tsx` usa Phosphor Icons (MIT), atribuido en THIRD_PARTY_NOTICES.md.
- Revisa licencias, fuentes y avisos de toda dependencia nueva según deny.toml y
  scripts/dependency-inventory.mjs; esas políticas no sustituyen las atribuciones de distribución.
- Contratos y soporte reales: docs/CONTRACTS.md y docs/STATUS.md. No habilites controles
  de audio/jobs por la mera existencia de tipos. La distribución actual es Windows x86/x64.

## Commits y ramas

- Ramas cortas desde `main`, PR con CI en verde.
- Mensajes en imperativo ("Add…", "Fix…") con contexto en el cuerpo.
- Los cambios visibles para el usuario se añaden a `CHANGELOG.md` en
  `[Unreleased]`.

## Releases

Ver [docs/RELEASES.md](docs/RELEASES.md).
