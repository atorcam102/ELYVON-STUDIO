# Releases, firma y actualizaciones

Este documento explica cómo se compilan, firman y publican las versiones de
Elyvon Studio, y qué credenciales hay que configurar.

## Preview 0.5.0 (Windows x64 local; macOS ARM64 y Ubuntu x64 en Actions a petición)

[`v0.5.0-preview.1`](https://github.com/atorcam102/ELYVON-STUDIO/releases/tag/v0.5.0-preview.1) apunta al commit `7f16dc8` (efectos nativos, fases 12–13, esquema 6).
Por petición expresa del usuario, **macOS ARM64 y Ubuntu x64** se compilaron en GitHub Actions con
el workflow manual `build-platforms.yml` (solo `workflow_dispatch`, no publica nada):
[run 39152097068](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/39152097068), donde pasaron
486 tests de Rust y 56 de Vitest en `macos-latest` y `ubuntu-22.04` antes de empaquetar. **Windows x64**
se compiló en el portátil desde un worktree limpio en el mismo commit (`tauri build --target
x86_64-pc-windows-msvc --bundles nsis msi`, sin artefactos de updater): 486 tests de Rust, Vitest
56/56 y E2E nativo 18/18 contra el binario release. Los cinco ficheros se publicaron con
`gh release create` (pre-release, no «latest»; ningún workflow se disparó) y se volvieron a
descargar para comprobar los hashes.

```
915f6464ff2a5ea9860939d283d70c4bc8d1619fb7b41d2028be526ab85807e1  Elyvon-Studio-v0.5.0-macos-arm64.dmg
e20f1d167144da188aa8c2c67314fd4e24eb61fb97bcb86e311ae767a2021048  Elyvon-Studio-v0.5.0-ubuntu-amd64.AppImage
a2762aa08e8e251160bb6e806f057f0696d3d8166e4f74f762cfe42bdb88807b  Elyvon-Studio-v0.5.0-ubuntu-amd64.deb
4b5b0864201ffdca42a8f8feb7d069cfc390782d61a6da0d96c6c4cda0d00422  Elyvon-Studio-v0.5.0-windows-x64-setup.exe
f54ec75eead6c2228513f675adc0f74d67bd21a33e94851a75208aab73f6ac60  Elyvon-Studio-v0.5.0-windows-x64.msi
```

Sin Authenticode, macOS con firma ad hoc sin notarizar, sin firma de updater. Los paquetes de
macOS y Ubuntu no se han instalado en un equipo real. No incluye Windows x86/ARM64, macOS Intel ni
Ubuntu ARM64.

## Preview 0.4.1 (Windows x64, compilación local)

[`v0.4.1-windows-preview.1`](https://github.com/atorcam102/ELYVON-STUDIO/releases/tag/v0.4.1-windows-preview.1)
apunta al commit `1521006`: la 0.4.0 más la segunda pasada visual de la interfaz (iconos, menús
navegables con teclado, barras de los editores). Mismo procedimiento que la 0.4.0 (abajo): worktree
limpio, `npm run check` (Vitest 50/50), `npm run check:rust` (402 tests), E2E nativo 17/17 sobre
el binario debug y de nuevo 17/17 sobre el binario release empaquetado, cabeceras PE/MSI, versión
0.4.1 en el ejecutable, publicación con `gh release create` y comprobación de lo descargado.
Ningún workflow se ejecutó.

```
1e612d036c4c307d3cdb8a7cb7d4029cb0852107337a894b75460bdd47d141bd  Elyvon-Studio-v0.4.1-windows-x64-setup.exe
6654d577df12ba621f7300cbc583fceba59cb2a0fd0a61214a5166e512c9ae34  Elyvon-Studio-v0.4.1-windows-x64.msi
```

Sin Authenticode ni firma de updater; no se instaló el paquete en esta verificación. Solo Windows x64.

## Preview 0.4.0 (Windows x64, compilación local)

[`v0.4.0-windows-preview.1`](https://github.com/atorcam102/ELYVON-STUDIO/releases/tag/v0.4.0-windows-preview.1)
apunta al commit `b47b730` e incluye las fases 10–11 (automatización, comping, edición no
destructiva) y el rediseño de la interfaz. Siguiendo la política del repositorio (CLAUDE.md), se
compiló y verificó **en el portátil Windows 11 x64 del mantenedor**, sin GitHub Actions:

1. Worktree limpio en el commit exacto (sin cambios locales de otras sesiones); `npm ci`,
   `npm run check`, `npm run check:rust` (402 tests) y E2E nativo 17/17 sobre el binario debug.
2. `npm run tauri -- build --target x86_64-pc-windows-msvc --bundles nsis msi --config
   '{"bundle":{"createUpdaterArtifacts":false}}'` (sin clave de updater).
3. E2E nativo 17/17 contra `target/x86_64-pc-windows-msvc/release/elyvon-studio.exe`, el binario
   que empaquetan los instaladores (`ELYVON_APP=… npm run test:e2e`).
4. Renombrado canónico, cabeceras PE (`MZ`) y MSI (OLE), versión 0.4.0 en el ejecutable y
   `SHA256SUMS.txt`; `gh release create --prerelease` con esos tres archivos; descarga y
   `sha256sum -c` de lo publicado. Ningún workflow se ejecutó (push con `[skip ci]`; la etiqueta
   se crea con la release, no con `git push`).

```
9e0491acd7180e113b899494abcb1214eebf619f06741463cc9b67bae5b7d745  Elyvon-Studio-v0.4.0-windows-x64-setup.exe
3b3c20cc12b53255477f92740c2261ed0b12fed6f8287c0e11483a9dc2cfd0b3  Elyvon-Studio-v0.4.0-windows-x64.msi
```

Sin Authenticode ni firma de updater; no se instaló el paquete en esta verificación. Solo Windows x64.

## Preview 0.2.0 de audio y MIDI

El tag `v0.2.0-windows-preview.1` identifica la aplicación 0.2.0 con las fases
04–07 integradas. El workflow `Publish Windows preview` espera a que **CI** termine
con éxito en `main`, comprueba que el tag apunta exactamente al SHA de ese run y
verifica sus seis jobs. Descarga los cuatro instaladores x64/x86 de ese mismo run,
valida nombres, cabeceras EXE/MSI y hashes, crea un borrador y lo publica después
de descargar y verificar sus cinco archivos adjuntos. No recompila ni reutiliza
binarios de versiones anteriores. Un CI de otro commit no modifica el release.

Es una preview sin firma de updater ni Authenticode y no publica `latest.json`.
No incluye instaladores de macOS/Linux. Se conserva el flujo SemVer general para
otros lanzamientos; los pushes de tags `-windows-preview.*` siguen excluidos de
su matriz heredada de siete plataformas. Para reintentar la publicación se puede
ejecutar `Publish Windows preview` con el ID del CI correcto en `source_run`.

La consolidación de los dos motores que divergieron en main/gallant está
documentada en [ADR 0006](ADR/0006-converge-phase03-and-arrangement-engines.md).

## Estado auditado y alcance actual (2026-10-15)

La distribución autorizada actualmente es **Windows x64 y x86**. El workflow CI
compila esos dos targets, y prueba el ejecutable Linux x64 como infraestructura de tests.
Las siete entradas del workflow Release descrito abajo son configuración heredada;
no equivalen a siete targets compilados o probados. No se ha verificado instalación
en hardware Windows, macOS o Linux ARM. Véase [STATUS](STATUS.md).

La [Release Windows preview 3](https://github.com/atorcam102/ELYVON-STUDIO/releases/tag/v0.1.0-windows-preview.3)
contiene cuatro instaladores independientes EXE/MSI y sus hashes, del commit `974b09c`
y [CI 38768320136](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38768320136).
Se publicó y verificó mediante el [workflow 38770622156](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38770622156).
EXE (NSIS) y MSI (Windows Installer) contienen la misma aplicación por arquitectura;
solo se instala uno. Para la mayoría de equipos Windows de 64 bits se recomienda el x64 EXE. No lleva firma de
updater ni Authenticode. La clave pública embebida sigue en UNCONFIGURED; la existencia
de secretos privados del repositorio no se pudo comprobar (API 403), por lo que no se
presenta como ausente ni como configurada. El test con clave de producción sigue pendiente.

Los tags `v0.1.0-windows-preview.*` nombran binarios existentes de aplicación 0.1.0:
se publican por el flujo específico de previews, y el flujo SemVer de reconstrucción
omite esos pushes. Un dispatch explícito del flujo Release conserva su comprobación
estricta tag == versión. No ejecutes la matriz heredada de siete builds como parte de
una fase limitada a Windows; hay que adaptar y verificar ese lanzamiento cuando se autorice.

La política estable rechaza prereleases, pero aún no hay endpoint/canal beta independiente.
Firmar con éxito tampoco prueba una instalación/actualización en dispositivo real.

## Resumen

```
tag vX.Y.Z ──► Release workflow
   preflight  versión = tag, changelog, secretos, clave privada ↔ clave pública
   build ×7   instaladores firmados por plataforma/arquitectura (runners nativos)
   publish    latest.json completo + verificación de todas las firmas + SHA256SUMS
              ──► release en BORRADOR (pre-release si el tag lleva sufijo)
maintainer revisa el borrador y pulsa "Publish"
   ──► releases/latest/download/latest.json ofrece la nueva versión a los usuarios
```

Con la clave de firma configurada nunca se publica automáticamente una versión
estable: el workflow crea un borrador y los usuarios solo reciben la
actualización cuando un responsable lo publica.

**Modo sin clave (rama condicional del workflow heredado).** Si `TAURI_SIGNING_PRIVATE_KEY` no existe,
el workflow compila igualmente los siete instaladores, pero sin paquetes de
actualización ni `latest.json`, y los publica como **pre-release descargable**
con un aviso al principio de las notas. Esas compilaciones llevan
`pubkey = "UNCONFIGURED"`, por lo que no buscan actualizaciones y lo indican en
*Settings › Updates*; el endpoint `releases/latest/…` nunca sirve pre-releases.
En cuanto se configure la clave, la siguiente versión (p. ej. v0.1.1) sale por
el flujo firmado y desde ella las actualizaciones automáticas funcionan.

## Matriz de compilación

| Etiqueta | Runner | Target | Artefactos |
| --- | --- | --- | --- |
| windows-x64 | `windows-latest` | `x86_64-pc-windows-msvc` | `-setup.exe`, `.msi` |
| windows-x86 | `windows-latest` | `i686-pc-windows-msvc` | `-setup.exe`, `.msi` |
| windows-arm64 | `windows-11-arm` (nativo) | `aarch64-pc-windows-msvc` | `-setup.exe` (WiX/MSI no soporta ARM64) |
| macos-intel | `macos-15-intel` (nativo) | `x86_64-apple-darwin` | `.dmg`, `.app.tar.gz` |
| macos-arm64 | `macos-latest` (nativo) | `aarch64-apple-darwin` | `.dmg`, `.app.tar.gz` |
| ubuntu-amd64 | `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | `.deb`, `.AppImage` |
| ubuntu-arm64 | `ubuntu-22.04-arm` (nativo) | `aarch64-unknown-linux-gnu` | `.deb`, `.AppImage` |

Notas:

- **Windows x86** se compila en un runner x64 con la toolchain MSVC de 32 bits
  (no hay runners x86; no es emulación).
- Linux se compila sobre Ubuntu 22.04 para enlazar contra una glibc antigua y
  funcionar en 22.04 y posteriores.
- Los runners ARM de GitHub son gratuitos para repositorios públicos. Si el
  repositorio pasa a privado, `windows-11-arm` y `ubuntu-22.04-arm` requieren un
  plan con runners de pago o runners propios; alternativa documentada:
  compilación cruzada de `aarch64-pc-windows-msvc` desde `windows-latest`
  (solo NSIS).
- El job *Build* del workflow **CI** compila Windows x64 y x86 sin firmar en cada
  push. El resto de esta tabla corresponde al flujo Release heredado, sin evidencia
  de compilación para los otros cinco targets en esta auditoría.

## Nombres de archivo

Los define `elyvon-update` (`ArtifactNaming`), y los usan tanto el pipeline
(`elyvon-release name`, `scripts/collect-artifacts.mjs`) como la aplicación al
validar una descarga:

```
Elyvon-Studio-v0.1.0-windows-x64-setup.exe   (+ .sig)
Elyvon-Studio-v0.1.0-windows-x64.msi         (+ .sig)
Elyvon-Studio-v0.1.0-windows-x86-setup.exe   (+ .sig)
Elyvon-Studio-v0.1.0-windows-x86.msi         (+ .sig)
Elyvon-Studio-v0.1.0-windows-arm64-setup.exe (+ .sig)
Elyvon-Studio-v0.1.0-macos-intel.dmg
Elyvon-Studio-v0.1.0-macos-intel.app.tar.gz  (+ .sig)   ← paquete de actualización
Elyvon-Studio-v0.1.0-macos-arm64.dmg
Elyvon-Studio-v0.1.0-macos-arm64.app.tar.gz  (+ .sig)
Elyvon-Studio-v0.1.0-ubuntu-amd64.deb        (+ .sig)
Elyvon-Studio-v0.1.0-ubuntu-amd64.AppImage   (+ .sig)
Elyvon-Studio-v0.1.0-ubuntu-arm64.deb        (+ .sig)
Elyvon-Studio-v0.1.0-ubuntu-arm64.AppImage   (+ .sig)
latest.json                                    ← metadatos del updater
SHA256SUMS.txt
```

## `latest.json`

Formato estático del plugin oficial de Tauri, con **una entrada por
`{os}-{arch}-{instalador}`** y sin entradas genéricas `{os}-{arch}`, para que
una instalación nunca pueda resolver un paquete de otra arquitectura o de otro
tipo de instalador:

```json
{
  "version": "0.1.0",
  "notes": "…sección del CHANGELOG…",
  "pub_date": "2026-10-15T12:00:00Z",
  "platforms": {
    "windows-x86_64-nsis": { "url": "https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0/Elyvon-Studio-v0.1.0-windows-x64-setup.exe", "signature": "…" },
    "windows-i686-msi": { … },
    "darwin-aarch64-app": { … },
    "linux-x86_64-deb": { … },
    "linux-aarch64-appimage": { … }
  }
}
```

`elyvon-release manifest … --require-complete --verify-config src-tauri/tauri.conf.json`
se niega a escribir el manifiesto si falta alguna de las 11 entradas, si una URL
no es HTTPS o no apunta al archivo exacto de esa plataforma y versión, o si
**alguna firma no verifica con la clave pública embebida en la aplicación** (y
vinculada a la versión).

## Cómo decide la aplicación

1. `tauri-plugin-updater` descarga `latest.json` desde
   `https://github.com/atorcam102/ELYVON-STUDIO/releases/latest/download/latest.json`
   (TLS obligatorio; GitHub excluye borradores y pre-releases de `latest`).
2. El tipo de instalador lo graba el empaquetador en el binario; el plugin busca
   `{os}-{arch}-{instalador}` (p. ej. `linux-x86_64-deb`). La arquitectura es la
   del binario compilado, no la del equipo: una app x86 en Windows x64 sigue
   recibiendo paquetes x86.
3. `elyvon_update::decide`: solo versiones mayores (SemVer, sin degradaciones);
   una instalación estable nunca pasa a una pre-release.
4. Comprobación adicional: el archivo anunciado debe tener exactamente el nombre
   esperado para esa plataforma, instalador y versión.
5. El usuario ve versión instalada, nueva versión y notas; decide descargar
   (cancelable, con progreso); el plugin verifica la firma minisign y la versión
   firmada (`requireSignedVersion: true`).
6. Instalar y reiniciar exigen confirmación y que no haya cambios sin guardar.
   En Windows el instalador cierra la aplicación; en macOS/Linux se ofrece
   "Restart now".

Sin clave configurada, en compilaciones de desarrollo, o en copias no instaladas
desde un instalador, la aplicación **no comprueba** actualizaciones y lo explica
en *Settings › Updates*.

## Secretos necesarios (pendiente de configurar)

### 1. Clave de firma de actualizaciones (obligatoria)

Generar **una sola vez**, en un equipo de confianza (no en CI):

```bash
npx tauri signer generate -w ~/.tauri/elyvon-studio.key
# pide una contraseña; guarda la clave privada y la contraseña en un gestor seguro
```

- Copiar el contenido de `~/.tauri/elyvon-studio.key.pub` en
  `src-tauri/tauri.conf.json` → `plugins.updater.pubkey` (sustituye
  `UNCONFIGURED`) y hacer commit. La clave pública forma parte de la aplicación.
- En GitHub → *Settings › Secrets and variables › Actions* crear:
  - `TAURI_SIGNING_PRIVATE_KEY`: contenido de `~/.tauri/elyvon-studio.key`
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: su contraseña

La clave privada nunca entra en el repositorio ni se genera en CI. **Si se
pierde, las instalaciones existentes no podrán actualizarse**: guardar copia de
seguridad cifrada.

Comprobación local de que la clave privada corresponde a la pública:

```bash
cargo build --release -p elyvon-update --bin elyvon-release
echo test > probe.bin
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/elyvon-studio.key)" npx tauri signer sign --app-version 0.1.0 probe.bin
./target/release/elyvon-release verify --config src-tauri/tauri.conf.json --file probe.bin --sig probe.bin.sig --version 0.1.0
```

### 2. Firma y notarización en macOS (recomendada)

Sin estos secretos el pipeline produce una firma *ad-hoc* (funciona en Apple
Silicon, pero Gatekeeper avisa en el primer arranque y no hay notarización).

| Secreto | Contenido |
| --- | --- |
| `APPLE_CERTIFICATE` | Certificado *Developer ID Application* exportado como `.p12`, en base64 (`base64 -i cert.p12`) |
| `APPLE_CERTIFICATE_PASSWORD` | Contraseña del `.p12` |
| `APPLE_SIGNING_IDENTITY` | p. ej. `Developer ID Application: Elyvon (TEAMID)` |
| `APPLE_ID` | Apple ID usado para notarizar |
| `APPLE_PASSWORD` | Contraseña específica de app de ese Apple ID |
| `APPLE_TEAM_ID` | Team ID de Apple Developer |

Tauri firma con *hardened runtime* y notariza automáticamente cuando están
presentes.

### 3. Firma de código en Windows (pendiente, sin certificado)

Los instaladores de Windows **no están firmados con Authenticode** todavía;
SmartScreen mostrará "editor desconocido". Esto no afecta a la seguridad de las
actualizaciones (que se verifican con la clave minisign). Para firmar:

- Opción recomendada: Azure Trusted Signing o un certificado EV/OV en HSM en la
  nube. Configurar `bundle.windows.signCommand` en `tauri.conf.json` con la
  herramienta del proveedor (p. ej. `trusted-signing-cli`) y añadir sus
  credenciales como secretos.
- Opción clásica: certificado `.pfx` → `certificateThumbprint` +
  `timestampUrl` en `bundle.windows`.

## Publicar una versión

1. Actualizar la versión en `package.json`, `Cargo.toml` (`[workspace.package]`)
   y `src-tauri/tauri.conf.json` (`npm run check:versions` comprueba que
   coinciden) y añadir la sección `## [X.Y.Z]` en `CHANGELOG.md`.
2. Commit en `main` con CI en verde.
3. `git tag vX.Y.Z && git push origin vX.Y.Z`
   (pre-releases: `vX.Y.Z-beta.N`).
4. Esperar al workflow *Release*. Revisar el borrador: 13 instaladores/paquetes,
   sus `.sig`, `latest.json` y `SHA256SUMS.txt`.
5. Probar al menos un instalador por sistema. Publicar el borrador.
6. Las instalaciones existentes con la comprobación automática activada verán la
   actualización en el siguiente arranque.

Para reconstruir un tag existente: *Actions › Release › Run workflow* con el tag.

## Qué está verificado y qué no

- Localmente (Ubuntu x64): compilación de release, empaquetado `.deb` y
  `.AppImage` firmados con una clave **de prueba**, renombrado, ensamblado de
  `latest.json` con verificación de firmas y validación.
- En CI: estado de cada objetivo en el job *Build* del workflow CI.
- No verificado aún: el flujo completo de actualización entre dos versiones
  publicadas (requiere la clave real y dos releases), la notarización de macOS
  y la firma Authenticode.
