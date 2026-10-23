// The render engine: the one place that turns a RenderPlan into FFmpeg arguments.
//
// The desktop app and the headless CLI are two front doors to this same engine, so
// there is exactly one final renderer (ADR-001: "FFmpeg là final renderer
// authoritative"). The engine has no UI dependency: callers pass in the progress sink
// and read the cancel flag.
pub mod contracts;
pub mod engine;
pub mod probe;
pub mod process;

pub use contracts::{RenderPlan, RenderProgress, RenderRange, RenderRequest};
pub use engine::{
    render_segments, ProgressSink, RENDER_CANCELLED,
};
