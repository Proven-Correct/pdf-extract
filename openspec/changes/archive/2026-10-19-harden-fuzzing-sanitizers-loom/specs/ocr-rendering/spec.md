# Spec Delta

## MODIFIED Requirements

### Requirement: Hostile inputs render or hit a limit
The system SHALL either render each page of a hostile document or return `RenderError::Limit` for it, never crashing the process. A panic inside an image codec SHALL skip that image with an `image: decode failed` warning and let the page render (as specified by `error-model`). Any other panic in the render path SHALL propagate to Rust callers as a panic and SHALL reach Python callers as `InternalError` (as specified by `error-model`).

#### Scenario: Hostile fixture set
- **WHEN** every page of every hostile fixture is rendered at 36 dpi on a 512 KiB thread stack
- **THEN** each outcome is a raster or `RenderError::Limit`, with no panic and no stack overflow [R1; 0002 §2.2, §2.3, §4.7; ADR-0035]

#### Scenario: Codec panic while rendering
- **WHEN** a page draws an image whose codec panics, with the test-only codec panic hook armed
- **THEN** the page renders without that image, and the result carries one `image: decode failed` warning [R1; internal #76]
