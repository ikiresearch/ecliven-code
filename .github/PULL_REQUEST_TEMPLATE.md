## Resumen

<!-- Qué cambia y por qué. -->

## Tipo de cambio

- [ ] Funcionalidad nueva
- [ ] Corrección
- [ ] Documentación
- [ ] Mantenimiento / CI

## Verificación

- [ ] `cargo test --locked`
- [ ] `cargo clippy --all-targets --locked -- -D warnings`
- [ ] `cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings` (si toca código de plataforma)
- [ ] `cargo deny check` (si cambian dependencias)
- [ ] README actualizado si cambia el comportamiento o la configuración

## Impacto en seguridad

- [ ] No toca sandbox, rutas, aprobaciones, Workspace Trust, procesos externos, instaladores ni CI
- [ ] Sí los toca: lo explico abajo y hay tests que cubren el caso

<!-- Explicación, si aplica. -->
