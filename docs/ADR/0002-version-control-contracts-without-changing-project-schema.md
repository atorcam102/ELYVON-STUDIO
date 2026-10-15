# ADR 0002: Version control contracts without changing project schema

Status: Accepted for this implementation · 2026-10-15.

Context: Phase 02 needs explicit MIDI/graph/job boundaries, while existing .elyvon schema-1
files must survive. Decision: add version-1 descriptive types with validation, generated TS
bindings, frozen wire fixtures and app_contracts. Do not add them to Project or enable engines.
Graph validation allocates off the RT thread, uses acyclic channel routing and rejects invalid
plans. Consequence: schema stays 1; future capabilities need implementation, negotiated contract
changes and independent project migrations where appropriate. See ../CONTRACTS.md.
