// The wire contract between the Rust shell and the TypeScript UI.
//
// The shared structs live in the render engine crate (crates/render-engine) because the
// app and the headless CLI are two front doors to the same engine and must speak the
// same contract. This module re-exports the pieces the app uses and keeps the ones
// that are app-only.
//
// The structs marked with the typeshare attribute are what `npm run contracts:generate`
// turns into packages/contracts/generated/generated.ts, and a parity test fails if the
// two sides drift apart.
pub(crate) use render_engine::contracts::{AnalysisStatus, ProbeMetadata};
