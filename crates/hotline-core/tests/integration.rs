//! One link for the harnesses that can safely share a process.
//! Explicit paths preserve the existing nested modules and fixture paths.

mod common;

#[path = "acp_shutdown.rs"]
mod acp_shutdown;
#[path = "acp_startup.rs"]
mod acp_startup;
#[path = "call_threads.rs"]
mod call_threads;
#[path = "desk.rs"]
mod desk;
#[path = "folders.rs"]
mod folders;
#[path = "local_speech.rs"]
mod local_speech;
#[path = "mcp.rs"]
mod mcp;
#[path = "rig_upgrade.rs"]
mod rig_upgrade;
#[path = "schedule.rs"]
mod schedule;
#[path = "steering.rs"]
mod steering;
#[path = "threads.rs"]
mod threads;
