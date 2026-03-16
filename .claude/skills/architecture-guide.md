---
name: architecture-guide
description: >
  Structure, conventions, and section-to-code mappings for docs/ARCHITECTURE.md.
  Loaded by agents during architecture revision to know where information
  belongs, which diagrams exist, and how to write consistent updates.
---

# Architecture Document Guide

Reference for maintaining `docs/ARCHITECTURE.md`.

## Document Structure (12 Sections)

| # | Section | Scope |
|---|---------|-------|
| 1 | Introduction & Crate Map | Workspace overview, crate table, dependency graph diagram |
| 2 | High-Level Architecture | Three-tier model (CLI/Manager/Server), overview diagram |
| 3 | Connection Lifecycle | Codec derivation, authentication, request/response phases; reconnection with OTP; reconnect strategies |
| 4 | Transport Layer | Transport trait, implementations, codec chain, key exchange, frame format |
| 5 | Authentication | Auth handshake sequence, server-side types, client-side handlers, sensitive data handling |
| 6 | Protocol | Serialization format, Msg wrapper, Request enum, Response enum, manager protocol, versioning |
| 7 | Plugin System | Plugin trait, registry, built-in plugins, external plugin binary protocol |
| 8 | The Api Trait & Backend Implementations | Api trait, ApiServerHandler, backend architecture pattern, per-backend details |
| 9 | Manager Architecture | ManagerServer struct, config, channel multiplexing, ManagerClient |
| 10 | CLI Command Tree | Command hierarchy, command-to-protocol mapping, shell session with predictive echo |
| 11 | Test Harness | Four test patterns (in-process, full CLI, SSH, Docker), cross-platform utilities, CI throttling |
| 12 | Key Type Reference | Core traits table, type aliases, configuration defaults, frame/buffer constants |

## Mermaid Diagram Inventory (14 Diagrams)

| # | Section | Type | Illustrates |
|---|---------|------|-------------|
| 1 | 1 | `flowchart TD` | Crate dependency graph |
| 2 | 2 | `flowchart LR` | High-level CLI/Manager/Server architecture |
| 3 | 3 | `sequenceDiagram` | Full connection lifecycle (codec → auth → request/response) |
| 4 | 3 | `sequenceDiagram` | Reconnection with OTP |
| 5 | 4 | `flowchart TB` | Transport layer stack (application → serialization → framing → codec → raw) |
| 6 | 4 | `sequenceDiagram` | ECDH P-256 key exchange |
| 7 | 5 | `sequenceDiagram` | Authentication handshake |
| 8 | 7 | `flowchart LR` | Plugin registry and built-in/external plugins |
| 9 | 8 | `flowchart LR` | Backend architecture pattern (Plugin → Api → Handler → Server → InmemoryTransport → Client) |
| 10 | 9 | `flowchart TB` | Manager daemon with connection pool and channels |
| 11 | 11 | `flowchart TB` | Four test patterns |
| 12-14 | — | (inline) | Various small diagrams within sections (frame format, etc.) |

## Writing Conventions

- **Tables** for comparisons, type listings, variant enumerations
- **Mermaid `flowchart`** for static relationships and architecture diagrams
- **Mermaid `sequenceDiagram`** for temporal flows and handshakes
- **Rust code blocks** for key type/trait signatures
- **Protocol variants grouped by domain** in tables (File I/O, Directory,
  Process, etc.)
- **Note defaults and constants** in the Key Type Reference section (Section 12)
- **ASCII art** only for the frame format diagram (Section 4)
- **Subgraph labels** in diagrams use quoted strings
- **Node labels** use `Name["Display<br/>text"]` format with `<br/>` for wrapping
- **Color styling** at bottom of flowcharts using `style node fill:#color`

## Section-to-Code Mapping

| Section | Primary Source Locations |
|---------|------------------------|
| 1. Crate Map | `Cargo.toml` (workspace), `*/Cargo.toml` |
| 2. High-Level Architecture | `distant-core/src/manager/`, `distant/src/cli/` |
| 3. Connection Lifecycle | `distant-core/src/net/transport/framed.rs`, `distant-core/src/net/transport/codec/`, `distant-core/src/net/keychain.rs` |
| 4. Transport Layer | `distant-core/src/net/transport/` (all), `distant-core/src/net/transport/codec/` |
| 5. Authentication | `distant-core/src/auth/`, `distant/src/cli/common/auth/` |
| 6. Protocol | `distant-core/src/protocol/`, `distant-core/src/manager/data/` |
| 7. Plugin System | `distant-core/src/plugin/`, `distant-core/src/manager/` |
| 8. Api & Backends | `distant-core/src/api/`, `distant-host/src/`, `distant-ssh/src/`, `distant-docker/src/` |
| 9. Manager | `distant-core/src/manager/`, `distant-core/src/manager/server/` |
| 10. CLI Commands | `distant/src/cli/commands/`, `distant/src/cli/commands/shell/` |
| 11. Test Harness | `distant-test-harness/src/`, `.config/nextest.toml` |
| 12. Key Types | Cross-cutting — all `lib.rs` files, `distant-core/src/net/`, `distant-core/src/api/` |

## Original Authoring Prompt

The following prompt created the initial `docs/ARCHITECTURE.md`. It captures
the document's intent and can inform future full rewrites:

> Your goal is to explore the full distant repository and create a comprehensive
> ARCHITECTURE.md file that describes the structs, traits, etc. at a high level
> that are exposed externally from each crate (distant-host, distant-ssh,
> distant-docker), the framework used to test distant end-to-end, and the
> distant CLI itself. This should include as many visuals as possible to describe
> how distant works at a high level, making use of mermaid js diagrams
> (https://mermaid.js.org/intro/) that are exposed as markdown to export via
> pandoc or some other markdown tool. Github itself supports mermaid:
> https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/creating-diagrams
>
> You can also read https://distant.dev/about/architecture/ that provides
> visuals to get an understanding.
>
> The idea is to compile this in a way that both humans and AI agents can get a
> faster understanding of how distant works from the CLI to the client -> manager
> -> server architecture, to the plugins (including the process plugin), to the
> underlying authentication that is used by distant-host and provided through
> transports (and document the transports). You need to figure out everything
> that should be documented. These are just examples.
