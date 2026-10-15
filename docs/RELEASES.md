# Releases, firma y actualizaciones

Este documento explica cómo se compilan, firman y publican las versiones de
Elyvon Studio, y qué credenciales hay que configurar.

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

**Modo sin clave (estado actual).** Si `TAURI_SIGNING_PRIVATE_KEY` no existe,
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
- El job *Build* del workflow **CI** compila los siete objetivos sin firmar en
  cada push: es la verificación de que cada plataforma compila.

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
