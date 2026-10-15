# ADR 0001: Rust owns session state and presentation stays in React

Status: Accepted for this implementation · 2026-10-15.

Context: Phase 1 already has native session validation, history and persistence. Replacing it
would risk data and duplicate the model. Decision: retain Rust as the sole project authority;
React sends EditCommand and renders SessionView. Pixel/layout/selection state stays in UI.
Audio/DSP and tick-to-sample scheduling belong to future Rust implementations. Consequence:
no frontend audio engine or fabricated worker/meter is introduced by this audit.
