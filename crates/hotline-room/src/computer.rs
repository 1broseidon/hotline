//! Where a teammate's computer puts things, for the code above the computer
//! that names them without starting one.

/// A named volume per teammate for the home itself, so what the teammate
/// prepared there outlives the container the hibernate cycle removes: the
/// environments it built for a workspace, its jobs and their output, its
/// shell history and its browser profile. The image keeps nothing of its
/// own in the home, so an empty volume is what a fresh container would
/// have had anyway. An extra mount may sit inside it; only one that would
/// cover it is refused, and that one already covers the workspace.
pub const HOME_MOUNT: &str = "/home/agent";
