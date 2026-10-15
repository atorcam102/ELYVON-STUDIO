# Contribuir a Elyvon Studio

## Entorno

| Herramienta | Versión |
| --- | --- |
| Rust | estable ≥ 1.88 (`rustup`) |
| Node.js | ≥ 22.12 |
| Linux | `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev libxdo-dev libssl-dev patchelf` |
| macOS | Xcode Command Line Tools |
| Windows | Visual Studio Build Tools (C++), WebView2 (incluido en Windows 10/11) |

```bash
npm ci
npm run tauri dev
```

## Comprobaciones (las mismas que CI)

```bash
npm run check:versions
npm run typecheck
npm run lint
npm test
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace         # también regenera src/bindings (hay que commitearlos)
```

Pruebas end-to-end contra el binario real (Linux):

```bash
sudo apt install webkit2gtk-driver xvfb
cargo install tauri-driver --locked
npx tauri build --debug --no-bundle
E2E_SCREENSHOTS=e2e-screenshots xvfb-run -a node --test e2e/app.e2e.mjs
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
- **Diseño**: usa los tokens de `src/styles/tokens.css`; iconos propios en
  `src/components/Icon.tsx` (no se usan recursos gráficos de terceros).
- Dependencias nuevas solo si aportan claramente; licencias MIT/Apache-2.0/BSD.

## Commits y ramas

- Ramas cortas desde `main`, PR con CI en verde.
- Mensajes en imperativo ("Add…", "Fix…") con contexto en el cuerpo.
- Los cambios visibles para el usuario se añaden a `CHANGELOG.md` en
  `[Unreleased]`.

## Releases

Ver [docs/RELEASES.md](docs/RELEASES.md).
