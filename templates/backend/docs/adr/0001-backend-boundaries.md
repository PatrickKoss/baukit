# ADR 0001: Enforce hexagonal backend boundaries with Cargo crates

Status: accepted

The backend separates domain, ports, services, HTTP, PostgreSQL, and process composition into workspace crates. Domain code has no framework dependency, services depend on ports, adapters implement ports, and the binary crate alone chooses concrete adapters and owns process lifecycle.
