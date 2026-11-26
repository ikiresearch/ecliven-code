# Contribuir a Ecliven Code

Gracias por ayudar. Esta guía resume cómo preparar el entorno y qué se revisa en cada PR.

## Entorno

```bash
git clone https://github.com/ikiresearch/ecliven-code
cd ecliven-code
cargo build
cargo test
```

Rust estable reciente. Para revisar la compilación de Windows desde Linux:

```bash
rustup target add x86_64-pc-windows-gnu
# El SQLite incluido necesita la API de Windows Vista o superior al compilar con MinGW.
CFLAGS_x86_64_pc_windows_gnu="-D_WIN32_WINNT=0x0A00" \
  cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings
```

## Antes de abrir un PR

```bash
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo deny check          # licencias, avisos de seguridad y orígenes de dependencias
```

CI ejecuta lo mismo en Linux y Windows, y `main` solo acepta cambios por PR con todos los checks en verde.

## Ramas y commits

- Ramas: `feat/<tema>`, `fix/<tema>`, `docs/<tema>`, `chore/<tema>`.
- Un PR por tema, con tests para el comportamiento nuevo o corregido.
- Mensajes en imperativo y concretos: `Add workspace trust for MCP servers`.

## Cambios sensibles

Estas áreas necesitan tests específicos y una explicación de impacto en el PR:

- `src/tools/policy.rs`, `src/tools/sandbox.rs`, `src/tools/mod.rs` (política central, sandbox y resolución de rutas);
- `src/config/trust.rs` y la carga de `.ecliven.toml` (Workspace Trust);
- `src/ai/cli_backend.rs` (procesos externos);
- `src/swarm/workcopy.rs` y `src/swarm/team.rs` (copias aisladas y aplicación de cambios);
- `install.sh`, `install.ps1` y `.github/workflows/`.

No incluyas secretos en código, tests ni ejemplos: usa valores como `test/model` o `/tmp/example`.

## Dependencias

- Prefiere crates mantenidos y con licencias permitidas en `deny.toml`.
- Cualquier dependencia nueva debe pasar `cargo deny check`.
- Dependabot propone actualizaciones semanales de crates y GitHub Actions.

## Seguridad

Las vulnerabilidades se reportan en privado, nunca en issues: ver [SECURITY.md](SECURITY.md).

## Licencia

Salvo que indiques lo contrario, cualquier contribución que envíes se licencia como `MIT OR Apache-2.0`, igual que el proyecto, sin términos adicionales.

## Conducta

Este proyecto sigue el [Código de conducta](CODE_OF_CONDUCT.md).
